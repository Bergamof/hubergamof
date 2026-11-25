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
		connection.receive({ type: 'updateStatus', status: { type: 'upToDate' } } as never);
		await render(ManagerHarness, { deck, page: HomePage });
		await expect.element(page.getByText(/^À jour\.$/)).toBeVisible();
	});
});
