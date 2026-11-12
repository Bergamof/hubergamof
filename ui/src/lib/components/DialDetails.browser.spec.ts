import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ManagerApi } from '#lib/api/manager-api.js';
import type {
	CatalogModule,
	DeviceConfig,
	DialAssignment,
	ModuleInstance
} from '#lib/generated/index.js';
import DialDetails from './DialDetails.svelte';

function module(kind: string, name: string, functionName: string): CatalogModule {
	return {
		kind,
		name,
		description: '',
		capabilities: { key: true },
		dialFunctions: [{ id: 'main', name: functionName, description: `Réglage de ${name}.` }],
		icon: null,
		settingIcons: {},
		defaultSettings: {},
		settings: []
	};
}

const MODULES = [
	module('audio', 'Audio', 'Contrôle du volume'),
	module('pomodoro', 'Pomodoro', 'Contrôle du pomodoro')
];

function instance(kind: string): ModuleInstance {
	return { id: kind, kind, label: null, image: null, settings: {} };
}

const VOLUME: DialAssignment = { kind: 'module', instance: 'audio', function: 'main' };

const CONFIG = {
	serial: 'DECK',
	home: [],
	dials: [VOLUME],
	modules: [instance('audio')]
} as unknown as DeviceConfig;

async function show(assignment: DialAssignment = VOLUME) {
	const onassign = vi.fn();
	await render(DialDetails, {
		index: 0,
		config: CONFIG,
		serial: 'DECK',
		api: {} as ManagerApi,
		assignment,
		instance: assignment.kind === 'module' ? instance('audio') : null,
		candidates: [{ instance: instance('audio'), place: 'molette 1' }],
		modules: MODULES,
		moduleOf: (kind: string) => MODULES.find((candidate) => candidate.kind === kind),
		headingId: 'details-heading',
		onassign,
		onapply: vi.fn()
	});
	return { onassign };
}

describe('DialDetails', () => {
	it('names the dial and offers the functions, the current one selected', async () => {
		await show();
		await expect.element(page.getByRole('heading', { name: 'Molette 1' })).toBeVisible();
		const functions = page.getByRole('combobox', { name: 'Fonction' });
		await expect.element(functions).toHaveValue('function:audio/main');
		for (const name of ['Rien', 'Navigation', 'Fonction contextuelle']) {
			await expect.element(page.getByRole('option', { name, exact: true })).toBeInTheDocument();
		}
		await expect
			.element(page.getByRole('option', { name: 'Contrôle du volume (Audio, molette 1)' }))
			.toBeInTheDocument();
		await expect
			.element(page.getByRole('option', { name: 'Contrôle du pomodoro (Pomodoro, nouveau)' }))
			.toBeInTheDocument();
	});

	it('assigns a function only when the button is pressed, never by browsing the list', async () => {
		const { onassign } = await show();
		const assign = page.getByRole('button', { name: 'Affecter' });
		await expect.element(assign).toHaveAttribute('aria-disabled', 'true');
		await page
			.getByRole('combobox', { name: 'Fonction' })
			.selectOptions('Contrôle du pomodoro (Pomodoro, nouveau)');
		expect(onassign).not.toHaveBeenCalled();
		await expect.element(assign).toHaveAttribute('aria-disabled', 'false');
		await assign.click();
		expect(onassign).toHaveBeenCalledOnce();
		expect(onassign.mock.calls[0]?.[0]).toMatchObject({
			type: 'new',
			module: { kind: 'pomodoro' },
			function: 'main'
		});
	});

	it('assigns a function of the manager', async () => {
		const { onassign } = await show();
		await page.getByRole('combobox', { name: 'Fonction' }).selectOptions('Navigation');
		await page.getByRole('button', { name: 'Affecter' }).click();
		expect(onassign).toHaveBeenCalledWith({ type: 'navigation' });
	});

	it('says what the function being chosen does', async () => {
		await show({ kind: 'navigation' });
		await expect.element(page.getByText(/fait défiler les lignes de la page/)).toBeVisible();
		await page.getByRole('combobox', { name: 'Fonction' }).selectOptions('Fonction contextuelle');
		await expect.element(page.getByText(/dépend de ce qui est affiché/)).toBeVisible();
		await page
			.getByRole('combobox', { name: 'Fonction' })
			.selectOptions('Contrôle du volume (Audio, molette 1)');
		await expect.element(page.getByText(/Réglage de Audio\./)).toBeVisible();
	});

	it('warns that a module placed nowhere else is deleted with its settings', async () => {
		await show();
		await page.getByRole('combobox', { name: 'Fonction' }).selectOptions('Rien');
		await expect.element(page.getByText(/« Audio » n’est placé nulle part ailleurs/)).toBeVisible();
	});

	it('has no settings to edit for a function of the manager', async () => {
		await show({ kind: 'navigation' });
		await expect
			.element(page.getByRole('combobox', { name: 'Fonction' }))
			.toHaveValue('navigation');
		await expect.element(page.getByLabelText('Libellé')).not.toBeInTheDocument();
	});

	it('edits the settings of the module of the function', async () => {
		await show();
		await expect.element(page.getByText(/Réglages de « Audio »/)).toBeVisible();
		await expect.element(page.getByLabelText('Libellé')).toBeVisible();
	});
});
