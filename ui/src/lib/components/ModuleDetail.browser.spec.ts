import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { ManagerApi } from '#lib/api/manager-api.js';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import type { ApplicationModuleView, CatalogModule } from '#lib/generated/index.js';
import ModuleDetail from './ModuleDetail.svelte';
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

async function renderPanel(
	modules: ApplicationModuleView[] = [view()],
	catalog: CatalogModule[] = [POMODORO_WITH_SETTINGS]
) {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	connection.receive({ type: 'moduleCatalog', modules: catalog });
	connection.receive({ type: 'applicationModules', modules });
	await render(ModuleDetail, { deck, api: new ManagerApi(), view: modules[0] });
	return { connection, deck };
}

describe('ModuleDetail', () => {
	it('shows the step and the time of the pomodoro, whether or not a device is connected', async () => {
		await renderPanel();
		await expect.element(page.getByRole('heading', { name: 'Pomodoro', level: 2 })).toBeVisible();
		await expect.element(page.getByText(/^Focus 2\/4$/).first()).toBeVisible();
		await expect.element(page.getByText(/^12:34 · pause$/)).toBeVisible();
		await expect.element(page.getByText(/Sessions de travail et pauses\./)).toBeVisible();
	});

	it('announces the step, not the time', async () => {
		await renderPanel();
		const step = page.getByRole('status').first();
		await expect.element(step).toHaveTextContent('Focus 2/4');
		expect(step.element().textContent).not.toContain('12:34');
	});

	it('says so when an action cannot be sent', async () => {
		const { connection, deck } = await renderPanel();
		connection.open = false;
		await page.getByRole('button', { name: 'Démarrer ou mettre en pause' }).click();
		expect(deck.notices).toHaveLength(1);
		expect(deck.notices[0].text).toMatch(/Action non envoyée/);
	});

	it('offers the actions of the module and sends the one that is chosen', async () => {
		const { connection } = await renderPanel();
		const actions = page.getByRole('group', { name: 'Actions de Pomodoro' });
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

	it('shows nothing of a module without a state', async () => {
		await renderPanel([view({ status: null as unknown as undefined })]);
		expect(document.querySelector('[data-testid="module-status-pomodoro"]')).toBeNull();
		await expect.element(page.getByRole('heading', { name: 'Pomodoro' })).toBeVisible();
	});

	describe('Teams, whose actions follow the call', () => {
		const action = (id: string, name: string) => ({ id, name, description: `${name}.` });
		const TEAMS: CatalogModule = {
			...POMODORO_WITH_SETTINGS,
			kind: 'teams',
			name: 'Teams',
			description: '',
			settings: [],
			defaultSettings: {},
			radialActions: [
				action('mute', 'Micro'),
				action('camera', 'Caméra'),
				action('hang-up', 'Raccrocher'),
				action('return-to-call', 'Revenir à l’appel'),
				action('conversations', 'Conversations'),
				action('teams', 'Équipes'),
				action('activity', 'Activité')
			]
		};
		const teams = (availableActions: string[] | null) =>
			({ kind: 'teams', status: null, settings: {}, availableActions }) as ApplicationModuleView;

		it('offers the controls of the call during a call, apart from the panels, hang up as the one that ends', async () => {
			await renderPanel(
				[
					teams([
						'mute',
						'camera',
						'hang-up',
						'return-to-call',
						'conversations',
						'teams',
						'activity'
					])
				],
				[TEAMS]
			);
			await expect.element(page.getByRole('heading', { name: 'Appel en cours' })).toBeVisible();
			await expect.element(page.getByRole('heading', { name: 'Ouvrir dans Teams' })).toBeVisible();
			const hangUp = page.getByRole('button', { name: 'Raccrocher' });
			await expect.element(hangUp).toBeVisible();
			expect(hangUp.element().className).toContain('danger-button');
			expect(page.getByRole('button', { name: 'Micro' }).element().className).toContain(
				'primary-button'
			);
		});

		it('offers only the panels when Teams is open and nobody calls', async () => {
			await renderPanel([teams(['conversations', 'teams', 'activity'])], [TEAMS]);
			await expect.element(page.getByRole('button', { name: 'Conversations' })).toBeVisible();
			await expect
				.element(page.getByRole('button', { name: 'Raccrocher' }))
				.not.toBeInTheDocument();
			await expect.element(page.getByRole('button', { name: 'Micro' })).not.toBeInTheDocument();
			await expect
				.element(page.getByRole('heading', { name: 'Appel en cours' }))
				.not.toBeInTheDocument();
		});

		it('says why there is nothing to do when Teams is not connected', async () => {
			await renderPanel([teams([])], [TEAMS]);
			await expect.element(page.getByText(/Aucune action pour l’instant/)).toBeVisible();
			await expect.element(page.getByRole('button')).not.toBeInTheDocument();
		});
	});
});
