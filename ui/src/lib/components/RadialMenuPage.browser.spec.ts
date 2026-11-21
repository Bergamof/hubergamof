import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import type { MouseConfig } from '#lib/generated/index.js';
import RadialMenuPage from './RadialMenuPage.svelte';
import { CATALOG, mouseConfig, mouseInfo, radialMenu } from './mouse-fixtures.js';

async function renderPage(options: { config?: MouseConfig; withMenu?: boolean } = {}) {
	const { config = mouseConfig(), withMenu = true } = options;
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	connection.receive({ type: 'moduleCatalog', modules: CATALOG });
	connection.receive({ type: 'mice', mice: [mouseInfo()] });
	connection.receive({ type: 'mouseConfig', serial: 'mx-1', config });
	if (withMenu) connection.receive({ type: 'radialMenu', config: radialMenu() });
	await render(RadialMenuPage, { deck, mouse: deck.mice.mouse, config: deck.mice.config });
	return { connection };
}

describe('RadialMenuPage', () => {
	it('shows a petal for each entry shown, and says what a click on one does', async () => {
		await renderPage();
		const preview = page.getByRole('group', { name: 'Aperçu du menu' });
		await expect.element(preview).toBeVisible();
		const petals = document.querySelectorAll('[aria-label="Aperçu du menu"] [role=button]');
		expect(petals.length).toBeGreaterThan(0);
		await expect.element(page.getByText(/Choisissez un pétale/)).toBeVisible();
		await preview.getByRole('button').first().click();
		await expect.element(page.getByText(/Un clic/)).toBeVisible();
	});

	it('tells which button opens the menu on the mouse', async () => {
		await renderPage({
			config: mouseConfig({ bindings: { gesture: { kind: 'radialMenu' } } })
		});
		await expect.element(page.getByText(/Sur MX Master 4, il s’ouvre avec/)).toBeVisible();
		expect(page.getByTestId('menu-trigger').element().textContent).toContain('Bouton de gestes');
	});

	it('says when no button opens the menu', async () => {
		await renderPage({ config: mouseConfig({ bindings: {} }) });
		await expect.element(page.getByText(/aucun bouton ne l’ouvre/)).toBeVisible();
	});

	it('sends a change of the menu and says when it was saved', async () => {
		const { connection } = await renderPage();
		await page.getByRole('switch', { name: /Animer/ }).click();
		expect(connection.sent.at(-1)).toMatchObject({
			type: 'updateRadialMenu',
			config: { animate: false }
		});
		await expect.element(page.getByText('Enregistrement…').first()).toBeVisible();
	});

	it('waits for the menu before showing it', async () => {
		await renderPage({ withMenu: false });
		await expect.element(page.getByText('Chargement du menu…')).toBeVisible();
	});
});
