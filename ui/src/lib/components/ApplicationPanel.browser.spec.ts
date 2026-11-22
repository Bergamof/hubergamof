import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { ManagerApi } from '#lib/api/manager-api.js';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import type { ApplicationModuleView, CatalogModule } from '#lib/generated/index.js';
import ApplicationPanel from './ApplicationPanel.svelte';
import { POMODORO } from './mouse-fixtures.js';

const POMODORO_WITH_SETTINGS: CatalogModule = {
	...POMODORO,
	description: 'Sessions de travail et pauses.',
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

function view(overrides: Partial<ApplicationModuleView> = {}): ApplicationModuleView {
	return {
		kind: 'pomodoro',
		status: 'Focus 2/4\n12:34 · pause',
		settings: { workMinutes: 25 },
		...overrides
	};
}

async function renderPanel(modules: ApplicationModuleView[] = [view()]) {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	connection.receive({ type: 'moduleCatalog', modules: [POMODORO_WITH_SETTINGS] });
	connection.receive({ type: 'applicationModules', modules });
	await render(ApplicationPanel, { deck, api: new ManagerApi() });
	return { connection, deck };
}

describe('ApplicationPanel', () => {
	it('shows the step and the time of the pomodoro, whether or not a device is connected', async () => {
		await renderPanel();
		await expect.element(page.getByRole('heading', { name: 'Pomodoro', level: 2 })).toBeVisible();
		await expect.element(page.getByText(/^Focus 2\/4$/)).toBeVisible();
		await expect.element(page.getByText(/^12:34 · pause$/)).toBeVisible();
		await expect.element(page.getByText(/Ce module tourne dans le gestionnaire/)).toBeVisible();
	});

	it('follows what the module shows as it changes', async () => {
		const { connection } = await renderPanel();
		connection.receive({
			type: 'applicationModules',
			modules: [view({ status: 'Focus 2/4\n12:33' })]
		});
		await expect.element(page.getByText(/^12:33$/)).toBeVisible();
	});

	it('offers the actions of the module and sends the one that is chosen', async () => {
		const { connection } = await renderPanel();
		const actions = page.getByRole('list', { name: 'Actions de Pomodoro' });
		await expect
			.element(actions.getByRole('button', { name: 'Démarrer ou mettre en pause' }))
			.toBeVisible();
		await actions.getByRole('button', { name: /Passer à l’étape suivante/ }).click();
		expect(connection.lastSent()).toEqual({
			type: 'moduleAction',
			module: 'pomodoro',
			action: 'skip'
		});
	});

	it('saves the settings as they are edited', async () => {
		const { connection } = await renderPanel();
		const field = page.getByLabelText('Durée du travail');
		await field.fill('30');
		await field.element().blur();
		expect(connection.lastSent()).toEqual({
			type: 'updateModuleSettings',
			module: 'pomodoro',
			settings: { workMinutes: 30 }
		});
	});

	it('shows nothing of a module without a state, and no section without modules', async () => {
		await renderPanel([view({ status: null as unknown as undefined })]);
		expect(document.querySelector('[data-testid="module-status-pomodoro"]')).toBeNull();
		await expect.element(page.getByRole('heading', { name: 'Pomodoro' })).toBeVisible();
	});
});
