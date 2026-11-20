import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { DeckState, type DeckConnection } from '#lib/deck/deck-state.svelte.js';
import type { ClientMessage, MouseInfo, ServerMessage } from '#lib/generated/index.js';
import MouseEditor from './MouseEditor.svelte';
import { CATALOG, mouseConfig, mouseInfo, radialMenu } from './mouse-fixtures.js';

class FakeConnection implements DeckConnection {
	readonly sent: ClientMessage[] = [];
	#listener: ((message: ServerMessage) => void) | undefined;

	send(message: ClientMessage): boolean {
		this.sent.push(message);
		return true;
	}

	onMessage(listener: (message: ServerMessage) => void): () => void {
		this.#listener = listener;
		return () => (this.#listener = undefined);
	}

	receive(message: ServerMessage): void {
		this.#listener?.(message);
	}
}

/** A state following one mouse, its configuration and the menu (unless `withMenu` is false). */
function connected(mouse: MouseInfo, withMenu = true) {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	connection.receive({ type: 'moduleCatalog', modules: CATALOG });
	connection.receive({ type: 'mice', mice: [mouse] });
	connection.receive({ type: 'mouseConfig', serial: mouse.serial, config: mouseConfig() });
	if (withMenu) connection.receive({ type: 'radialMenu', config: radialMenu() });
	return { connection, deck };
}

async function renderEditor(mouse: MouseInfo = mouseInfo(), withMenu = true) {
	const { connection, deck } = connected(mouse, withMenu);
	await render(MouseEditor, {
		deck,
		mouse: deck.mice.mouse!,
		config: deck.mice.config!,
		radialMenu: deck.mice.radialMenu
	});
	return { connection, deck };
}

describe('MouseEditor', () => {
	it('lists the buttons of the model, locked ones included, and leaves the wheel out of them', async () => {
		await renderEditor();
		await expect.element(page.getByRole('group', { name: 'Clic gauche' })).toBeVisible();
		await expect.element(page.getByRole('group', { name: 'Bouton arrière' })).toBeVisible();
		expect(document.body.textContent).not.toContain('Action de : Molette du pouce');
	});

	it('shows the vibrations of a haptic model only', async () => {
		await renderEditor();
		await expect.element(page.getByRole('heading', { name: 'Vibrations' })).toBeVisible();
	});

	it('leaves the vibrations out of a model that cannot play them', async () => {
		const info = mouseInfo();
		await renderEditor({ ...info, capabilities: { ...info.capabilities, haptics: false } });
		expect(document.body.textContent).not.toContain('Force des vibrations');
	});

	it('sends a change at once and says when the manager has saved it', async () => {
		const { connection } = await renderEditor();
		await page.getByLabelText('Action de : Bouton arrière').selectOptions('doubleClick');
		await expect.element(page.getByText('Enregistrement…')).toBeVisible();
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
		await expect.element(page.getByText('Modifications enregistrées.')).toBeVisible();
	});

	it('says when a change was refused', async () => {
		const { connection } = await renderEditor();
		await page.getByLabelText('Force des vibrations').selectOptions('high');
		connection.receive({ type: 'error', code: 'invalidConfig', message: 'refused' });
		await expect.element(page.getByText('Dernière modification refusée.')).toBeVisible();
	});

	it('sends a change of the radial menu', async () => {
		const { connection } = await renderEditor();
		await page.getByRole('checkbox', { name: /Animer/ }).click();
		expect(connection.sent.at(-1)).toMatchObject({
			type: 'updateRadialMenu',
			config: { animate: false }
		});
	});

	it('waits for the radial menu before showing it', async () => {
		await renderEditor(mouseInfo(), false);
		await expect.element(page.getByText('Chargement du menu…')).toBeVisible();
	});
});
