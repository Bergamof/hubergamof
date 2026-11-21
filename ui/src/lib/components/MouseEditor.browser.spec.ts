import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import type { MouseInfo } from '#lib/generated/index.js';
import MouseEditor from './MouseEditor.svelte';
import { CATALOG, mouseConfig, mouseInfo } from './mouse-fixtures.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';

/** A state following one mouse and its configuration. */
function connected(mouse: MouseInfo) {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	connection.receive({ type: 'moduleCatalog', modules: CATALOG });
	connection.receive({ type: 'mice', mice: [mouse] });
	connection.receive({ type: 'mouseConfig', serial: mouse.serial, config: mouseConfig() });
	return { connection, deck };
}

async function renderEditor(mouse: MouseInfo = mouseInfo()) {
	const { connection, deck } = connected(mouse);
	await render(MouseEditor, {
		deck,
		mouse: deck.mice.mouse!,
		config: deck.mice.config!
	});
	return { connection, deck };
}

describe('MouseEditor', () => {
	it('lists the controls of the model, locked ones included, each with what it does', async () => {
		await renderEditor();
		const list = page.getByRole('list').first();
		await expect.element(list.getByRole('button', { name: /Clic gauche/ })).toBeVisible();
		await expect.element(list.getByRole('button', { name: /Précédent/ })).toBeVisible();
		await expect.element(list.getByRole('button', { name: /Molette du pouce/ })).toBeVisible();
	});

	it('shows the drawings of the model with a marker on each control', async () => {
		await renderEditor();
		await expect
			.element(page.getByRole('group', { name: /MX Master 4, vue de dessus/ }))
			.toBeVisible();
		await expect
			.element(page.getByRole('group', { name: /MX Master 4, côté pouce/ }))
			.toBeVisible();
		await expect
			.element(page.getByRole('button', { name: /^3, Molette \(clic\)/ }).first())
			.toBeVisible();
	});

	it('describes a locked control and a wheel that only turns instead of offering actions', async () => {
		await renderEditor();
		await page
			.getByRole('list')
			.first()
			.getByRole('button', { name: /Clic gauche/ })
			.click();
		await expect.element(page.getByText(/Ce bouton est verrouillé/)).toBeVisible();
		await page
			.getByRole('list')
			.first()
			.getByRole('button', { name: /Molette du pouce/ })
			.click();
		await expect.element(page.getByText(/Cette molette n’a pas d’action/)).toBeVisible();
		expect(document.body.textContent).not.toContain('Rétablir');
	});

	it('shows the name of the mouse, what it is connected to and its tabs', async () => {
		await renderEditor();
		await expect.element(page.getByRole('heading', { name: 'MX Master 4' })).toBeVisible();
		await expect.element(page.getByText('Connectée · Logi Bolt')).toBeVisible();
		await expect.element(page.getByText('Retour haptique', { exact: true })).toBeVisible();
		await expect
			.element(page.getByRole('tab', { name: 'Touches' }))
			.toHaveAttribute('aria-selected', 'true');
	});

	it('shows the battery once the manager has read it, and when it is low or charging', async () => {
		const { connection } = await renderEditor();
		expect(document.body.textContent).not.toContain('Batterie');
		connection.receive({
			type: 'mouseStatus',
			serial: 'mx-1',
			problems: [],
			battery: { percent: 76, charging: false }
		});
		await expect.element(page.getByText('Batterie 76 %', { exact: true })).toBeVisible();
		expect(page.getByText('Batterie 76 %').element().className).not.toContain('text-warning');
		connection.receive({
			type: 'mouseStatus',
			serial: 'mx-1',
			problems: [],
			battery: { percent: 12, charging: false }
		});
		await expect.element(page.getByText('Batterie 12 % · faible')).toBeVisible();
		expect(page.getByText('Batterie 12 % · faible').element().className).toContain('text-warning');
		connection.receive({
			type: 'mouseStatus',
			serial: 'mx-1',
			problems: [],
			battery: { percent: 12, charging: true }
		});
		await expect.element(page.getByText('Batterie 12 % · en charge')).toBeVisible();
		expect(page.getByText(/^Batterie 12 %/).element().className).not.toContain('text-warning');
	});

	it('shows the pointer settings in their tab', async () => {
		await renderEditor();
		expect(document.body.textContent).not.toContain('Sensibilité du pointeur');
		await page.getByRole('tab', { name: 'Pointeur et molette' }).click();
		await expect.element(page.getByLabelText('Sensibilité du pointeur')).toBeVisible();
		const tab = page.getByRole('tab', { name: 'Pointeur et molette' }).element();
		expect(page.getByRole('tabpanel').element().getAttribute('aria-labelledby')).toBe(tab.id);
	});

	it('shows the vibrations of a haptic model, in their tab', async () => {
		await renderEditor();
		await page.getByRole('tab', { name: 'Haptique' }).click();
		await expect
			.element(page.getByRole('radiogroup', { name: 'Intensité générale' }))
			.toBeVisible();
	});

	it('leaves the vibrations out of a model that cannot play them', async () => {
		const info = mouseInfo();
		await renderEditor({ ...info, capabilities: { ...info.capabilities, haptics: false } });
		expect(document.querySelectorAll('[role=tab]')).toHaveLength(2);
		expect(document.body.textContent).not.toContain('Haptique');
		expect(document.body.textContent).not.toContain('Retour haptique');
	});

	it('sends a change at once and says when the manager has saved it', async () => {
		const { connection } = await renderEditor();
		await page
			.getByRole('list')
			.first()
			.getByRole('button', { name: /Précédent/ })
			.click();
		await page.getByLabelText('Action', { exact: true }).selectOptions('doubleClick');
		await expect.element(page.getByText('Enregistrement…').first()).toBeVisible();
		expect(connection.sent.at(-1)).toMatchObject({
			type: 'updateMouseConfig',
			serial: 'mx-1',
			config: { bindings: { back: { kind: 'doubleClick' } } }
		});
		connection.receive({
			type: 'mouseConfig',
			serial: 'mx-1',
			config: mouseConfig({ bindings: { back: { kind: 'doubleClick' } }, revision: 1 })
		});
		await expect.element(page.getByText('Modifications enregistrées.').first()).toBeVisible();
	});

	it('says when a change was refused', async () => {
		const { connection } = await renderEditor();
		await page.getByRole('tab', { name: 'Haptique' }).click();
		await page.getByRole('radio', { name: 'Forte' }).click();
		connection.receive({ type: 'error', code: 'invalidConfig', message: 'refused' });
		await expect.element(page.getByText('Dernière modification refusée.').first()).toBeVisible();
	});

	it('tells what the mouse refused, and that the rest was applied', async () => {
		const { connection } = await renderEditor();
		expect(document.body.textContent).not.toContain('la souris n’a pas accepté');
		connection.receive({
			type: 'mouseStatus',
			serial: 'mx-1',
			problems: [{ setting: 'sensitivity', message: 'no answer within 500ms' }]
		});
		await expect
			.element(page.getByText(/la sensibilité du pointeur : no answer within 500ms/))
			.toBeVisible();
		await expect.element(page.getByText(/Les autres sont appliqués/)).toBeVisible();
		connection.receive({ type: 'mouseStatus', serial: 'mx-1', problems: [] });
		await expect.element(page.getByText(/Les autres sont appliqués/)).not.toBeInTheDocument();
	});
});
