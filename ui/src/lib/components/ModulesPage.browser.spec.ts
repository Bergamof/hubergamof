import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import type { ApplicationModuleView, CatalogModule } from '#lib/generated/index.js';
import ModulesPageHarness from './ModulesPageHarness.svelte';
import { POMODORO } from './mouse-fixtures.js';

const WITH_SETTINGS: CatalogModule = {
	...POMODORO,
	settings: [
		{
			key: 'workMinutes',
			label: 'Durée du travail',
			help: null,
			kind: { type: 'integer', default: 25, min: 1, max: 90, unit: 'min' }
		}
	],
	defaultSettings: { workMinutes: 25 }
};
const WITHOUT_SETTINGS: CatalogModule = {
	...POMODORO,
	kind: 'claude',
	name: 'Claude',
	settings: []
};
const AUDIO: CatalogModule = {
	...WITH_SETTINGS,
	kind: 'audio',
	name: 'Audio'
};

const view = (kind: string, status: string): ApplicationModuleView => ({
	kind,
	status,
	settings: { workMinutes: 25 }
});

async function renderPage(requested: string | null = null) {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	connection.receive({ type: 'moduleCatalog', modules: [WITH_SETTINGS, WITHOUT_SETTINGS, AUDIO] });
	connection.receive({
		type: 'applicationModules',
		modules: [
			view('pomodoro', 'Focus 1/4\n25:00'),
			view('claude', 'Session'),
			view('audio', 'Sortie')
		]
	});
	await render(ModulesPageHarness, { deck, requested });
}

describe('ModulesPage', () => {
	it('lists the modules that have settings, and none of the others', async () => {
		await renderPage();
		const list = page.getByRole('navigation', { name: 'Modules à régler' });
		await expect.element(list.getByRole('link', { name: /Pomodoro/ })).toBeVisible();
		await expect.element(list.getByRole('link', { name: /Audio/ })).toBeVisible();
		await expect.element(list.getByRole('link', { name: /Claude/ })).not.toBeInTheDocument();
	});

	it('shows the first module when the address names none', async () => {
		await renderPage();
		await expect.element(page.getByRole('heading', { name: 'Pomodoro', level: 2 })).toBeVisible();
		await expect
			.element(page.getByRole('link', { name: /Pomodoro/ }))
			.toHaveAttribute('aria-current', 'true');
	});

	it('shows the module the address names', async () => {
		await renderPage('audio');
		await expect.element(page.getByRole('heading', { name: 'Audio', level: 2 })).toBeVisible();
		await expect
			.element(page.getByRole('link', { name: /Audio/ }))
			.toHaveAttribute('aria-current', 'true');
	});

	it('falls back to the first module when the address names an unknown one', async () => {
		await renderPage('nope');
		await expect.element(page.getByRole('heading', { name: 'Pomodoro', level: 2 })).toBeVisible();
	});

	it('says so when no module has a setting', async () => {
		const connection = new FakeConnection();
		const deck = new DeckState(connection, { load: () => null, save: () => undefined });
		deck.listen();
		connection.receive({ type: 'moduleCatalog', modules: [WITHOUT_SETTINGS] });
		connection.receive({ type: 'applicationModules', modules: [view('claude', 'Session')] });
		await render(ModulesPageHarness, { deck, requested: null });
		await expect.element(page.getByText('Aucun module à régler')).toBeVisible();
	});
});
