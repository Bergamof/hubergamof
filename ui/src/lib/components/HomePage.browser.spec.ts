import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import HomePage from './HomePage.svelte';
import ManagerHarness from './ManagerHarness.svelte';
import { POMODORO } from './mouse-fixtures.js';

function manager() {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	connection.receive({ type: 'hello', version: '1.4.2' } as never);
	return { connection, deck };
}

describe('HomePage', () => {
	it('says when nothing is connected and no module lives in the application', async () => {
		const { deck } = manager();
		await render(ManagerHarness, { deck, page: HomePage });
		await expect.element(page.getByRole('heading', { name: 'Accueil', level: 1 })).toBeVisible();
		await expect.element(page.getByText(/Aucun périphérique connecté\./)).toBeVisible();
		await expect.element(page.getByText(/Aucun module dans l’application/)).toBeVisible();
	});

	it('shows what the modules say now', async () => {
		const { connection, deck } = manager();
		connection.receive({ type: 'moduleCatalog', modules: [POMODORO] });
		connection.receive({
			type: 'applicationModules',
			modules: [{ kind: 'pomodoro', status: 'Focus 2/4\n12:34 · pause', settings: {} }]
		});
		await render(ManagerHarness, { deck, page: HomePage });
		await expect.element(page.getByText(/^Focus 2\/4$/)).toBeVisible();
		await expect.element(page.getByText(/^12:34 · pause$/)).toBeVisible();
	});

	it('announces an update that is waiting, and offers the page of updates', async () => {
		const { connection, deck } = manager();
		connection.receive({
			type: 'updateStatus',
			status: { type: 'available', version: '1.5.0', notesUrl: 'https://example.org' }
		} as never);
		await render(ManagerHarness, { deck, page: HomePage });
		await expect.element(page.getByText(/Version 1\.5\.0 disponible/)).toBeVisible();
		await expect.element(page.getByRole('link', { name: 'Mettre à jour' })).toBeVisible();
	});

	it('names what the manager is when it is up to date', async () => {
		const { connection, deck } = manager();
		connection.receive({
			type: 'updateStatus',
			status: { type: 'upToDate', checkedAt: 1_700_000_000 }
		} as never);
		await render(ManagerHarness, { deck, page: HomePage });
		await expect.element(page.getByText(/^À jour\.$/)).toBeVisible();
	});

	it('shows the battery of every mouse, and offers to read the one of the headphones', async () => {
		const { connection, deck } = manager();
		connection.receive({
			type: 'mice',
			mice: [
				{
					serial: 'M1',
					model: 'mxMaster4',
					displayName: 'MX Master 4',
					capabilities: { controls: [], thumbWheel: true, haptics: true },
					battery: { percent: 78, charging: false }
				},
				{
					serial: 'M2',
					model: 'mxAnywhere3s',
					displayName: 'MX Anywhere 3S',
					capabilities: { controls: [], thumbWheel: true, haptics: true },
					battery: { percent: 14, charging: false }
				}
			]
		} as never);
		connection.receive({ type: 'headsets', headsets: [{ address: 'AA', name: 'WH-1000XM4' }] });
		await render(ManagerHarness, { deck, page: HomePage });
		await expect.element(page.getByText(/78\s*%/)).toBeVisible();
		await expect.element(page.getByText(/14\s*%\s*· faible/)).toBeVisible();
		await expect.element(page.getByText(/non lue/)).toBeVisible();
		connection.sent.length = 0;
		await page.getByRole('button', { name: /Lire la batterie/ }).click();
		expect(connection.sent).toContainEqual({ type: 'selectHeadset', address: 'AA' });
	});

	it('shows the last battery read of the headphones, and offers to read it again', async () => {
		const { connection, deck } = manager();
		connection.receive({ type: 'headsets', headsets: [{ address: 'AA', name: 'WH-1000XM4' }] });
		deck.headsets.enter();
		connection.receive({
			type: 'headsetState',
			address: 'AA',
			state: { link: 'connected', windReduction: true, battery: { percent: 55, charging: false } }
		} as never);
		deck.headsets.leave();
		await render(ManagerHarness, { deck, page: HomePage });
		await expect.element(page.getByText(/55\s*%/)).toBeVisible();
		await expect.element(page.getByRole('button', { name: /Relire la batterie/ })).toBeVisible();
	});

	it('warns that reading the battery cuts the Sony application, and keeps the button while it reads', async () => {
		const { connection, deck } = manager();
		connection.receive({ type: 'headsets', headsets: [{ address: 'AA', name: 'WH-1000XM4' }] });
		await render(ManagerHarness, { deck, page: HomePage });
		await expect.element(page.getByText(/l’application Sony se coupe/)).toBeVisible();
		const button = page.getByRole('button', { name: /Lire la batterie/ });
		await button.click();
		// Still there, said busy and announced; a second click asks nothing more.
		await expect
			.element(page.getByRole('button', { name: /Lecture/ }))
			.toHaveAttribute('aria-disabled', 'true');
		await expect.element(page.getByText('Lecture de la batterie du casque…')).toBeInTheDocument();
		connection.sent.length = 0;
		await page.getByRole('button', { name: /Lecture/ }).click({ force: true });
		expect(connection.sent).toEqual([]);
		// Read: announced, and the button is back for another reading.
		connection.receive({
			type: 'headsetState',
			address: 'AA',
			state: { link: 'connected', windReduction: true, battery: { percent: 70, charging: false } }
		} as never);
		await expect.element(page.getByText('Batterie du casque : 70 %.')).toBeInTheDocument();
		await expect.element(page.getByRole('button', { name: /Relire la batterie/ })).toBeVisible();
	});

	it('says when the link to the headphones could not be opened', async () => {
		const { connection, deck } = manager();
		connection.receive({ type: 'headsets', headsets: [{ address: 'AA', name: 'WH-1000XM4' }] });
		await render(ManagerHarness, { deck, page: HomePage });
		await page.getByRole('button', { name: /Lire la batterie/ }).click();
		connection.receive({
			type: 'headsetState',
			address: 'AA',
			state: { link: 'cut', windReduction: true }
		} as never);
		await expect.element(page.getByText(/n’a pas pu s’ouvrir/)).toBeInTheDocument();
		await expect.element(page.getByRole('button', { name: /Lire la batterie/ })).toBeVisible();
	});

	it('offers the action of a module only when it makes sense now', async () => {
		const { connection, deck } = manager();
		const teams = {
			...POMODORO,
			kind: 'teams',
			name: 'Teams',
			radialActions: [
				{ id: 'mute', name: 'Micro', description: '' },
				{ id: 'conversations', name: 'Conversations', description: '' }
			]
		};
		connection.receive({ type: 'moduleCatalog', modules: [teams] });
		connection.receive({
			type: 'applicationModules',
			modules: [{ kind: 'teams', status: null, settings: {}, availableActions: ['conversations'] }]
		} as never);
		await render(ManagerHarness, { deck, page: HomePage });
		// Not in a call: the first action of the module (mute) is not offered, the next one is.
		await expect.element(page.getByRole('button', { name: /^Conversations/ })).toBeVisible();
		await expect.element(page.getByRole('button', { name: /^Micro/ })).not.toBeInTheDocument();
	});
});
