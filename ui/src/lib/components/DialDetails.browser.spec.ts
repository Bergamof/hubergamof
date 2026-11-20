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
		capabilities: {
			key: true,
			multipleInstances: false,
			usableWhenLocked: false,
			lockSafePages: false
		},
		dialFunctions: [{ id: 'main', name: functionName, description: `Réglage de ${name}.` }],
		icon: null,
		settingIcons: {},
		defaultSettings: {},
		radialActions: [],
		hapticEvents: [],
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

async function show(assignment: DialAssignment = VOLUME, config: DeviceConfig = CONFIG, index = 0) {
	const onassign = vi.fn();
	const onswap = vi.fn();
	await render(DialDetails, {
		index,
		config,
		serial: 'DECK',
		api: {} as ManagerApi,
		assignment,
		instance: assignment.kind === 'module' ? instance('audio') : null,
		candidates: [instance('audio')],
		modules: MODULES,
		moduleOf: (kind: string) => MODULES.find((candidate) => candidate.kind === kind),
		headingId: 'details-heading',
		onassign,
		onswap,
		onapply: vi.fn(),
		ondiscard: vi.fn()
	});
	return { onassign, onswap };
}

describe('DialDetails', () => {
	const THREE_DIALS = { ...CONFIG, dials: [VOLUME, VOLUME, VOLUME] } as unknown as DeviceConfig;
	const previous = () => page.getByRole('button', { name: /Échanger avec la molette précédente/ });
	const next = () => page.getByRole('button', { name: /Échanger avec la molette suivante/ });

	it('swaps with the next dial, the previous one being missing on the first', async () => {
		const { onswap } = await show(VOLUME, THREE_DIALS, 0);
		await expect.element(previous()).toHaveAttribute('aria-disabled', 'true');
		await previous().click({ force: true });
		expect(onswap).not.toHaveBeenCalled();
		await next().click();
		expect(onswap).toHaveBeenCalledExactlyOnceWith(1);
	});

	it('swaps with the previous dial, the next one being missing on the last', async () => {
		const { onswap } = await show(VOLUME, THREE_DIALS, 2);
		await expect.element(next()).toHaveAttribute('aria-disabled', 'true');
		await next().click({ force: true });
		expect(onswap).not.toHaveBeenCalled();
		await previous().click();
		expect(onswap).toHaveBeenCalledExactlyOnceWith(1);
	});

	it('has nothing to swap with on a single dial', async () => {
		await show();
		await expect.element(previous()).toHaveAttribute('aria-disabled', 'true');
		await expect.element(next()).toHaveAttribute('aria-disabled', 'true');
	});

	it('names the dial and offers the functions, the current one selected', async () => {
		await show();
		await expect.element(page.getByRole('heading', { name: 'Molette 1' })).toBeVisible();
		const functions = page.getByRole('combobox', { name: 'Fonction' });
		await expect.element(functions).toHaveValue('function:audio/main');
		for (const name of ['Rien', 'Navigation', 'Fonction contextuelle']) {
			await expect.element(page.getByRole('option', { name, exact: true })).toBeInTheDocument();
		}
		const optionsOf = (group: string) =>
			Array.from(
				document.querySelectorAll(`optgroup[label="${group}"] option`),
				(option) => option.textContent
			);
		expect(optionsOf('Fonctions des modules configurés')).toEqual(['Contrôle du volume']);
		expect(optionsOf('Nouveau module (réglages à zéro)')).toContain('Contrôle du pomodoro');
		// Where a function lives is not repeated in its name: no « (module, touche 1) ».
		const names = Array.from(document.querySelectorAll('option'), (option) => option.textContent);
		expect(names.every((name) => !name?.includes('('))).toBe(true);
	});

	it('tells that the settings are shared with the key of a module that exists once', async () => {
		await show(VOLUME, { ...CONFIG, home: ['audio'] } as unknown as DeviceConfig);
		await expect.element(page.getByText(/réglages sont partagés avec sa touche/)).toBeVisible();
	});

	it('says nothing about sharing when the module is on no key', async () => {
		await show();
		await expect
			.element(page.getByText(/réglages sont partagés avec sa touche/))
			.not.toBeInTheDocument();
	});

	it('does not offer a module that another dial already has', async () => {
		const config = {
			...CONFIG,
			dials: [VOLUME, { kind: 'none' }],
			modules: [instance('audio'), instance('pomodoro')]
		} as unknown as DeviceConfig;
		await show({ kind: 'none' }, config, 1);
		await expect
			.element(page.getByRole('option', { name: 'Contrôle du volume' }))
			.not.toBeInTheDocument();
		await expect
			.element(page.getByRole('option', { name: 'Contrôle du pomodoro' }))
			.toBeInTheDocument();
	});

	it('assigns a function only when the button is pressed, never by browsing the list', async () => {
		const { onassign } = await show();
		const assign = page.getByRole('button', { name: 'Affecter' });
		await expect.element(assign).toHaveAttribute('aria-disabled', 'true');
		await page.getByRole('combobox', { name: 'Fonction' }).selectOptions('Contrôle du pomodoro');
		expect(onassign).not.toHaveBeenCalled();
		await expect.element(assign).toHaveAttribute('aria-disabled', 'false');
		await assign.click();
		await page.getByRole('button', { name: 'Retirer' }).click();
		expect(onassign).toHaveBeenCalledOnce();
		expect(onassign.mock.calls[0]?.[0]).toMatchObject({
			type: 'new',
			module: { kind: 'pomodoro' },
			function: 'main'
		});
	});

	it('assigns a function of the manager', async () => {
		const config = { ...CONFIG, dials: [{ kind: 'contextual' }] } as unknown as DeviceConfig;
		const { onassign } = await show({ kind: 'contextual' }, config);
		await page.getByRole('combobox', { name: 'Fonction' }).selectOptions('Navigation');
		await page.getByRole('button', { name: 'Affecter' }).click();
		expect(onassign).toHaveBeenCalledWith({ type: 'navigation' });
	});

	it('asks to confirm before unassigning a module placed nowhere else', async () => {
		const { onassign } = await show();
		const select = page.getByRole('combobox', { name: 'Fonction' });
		await select.selectOptions('Rien');
		await page.getByRole('button', { name: 'Affecter' }).click();
		const dialog = page.getByRole('dialog', { name: 'Retirer la fonction de la molette ?' });
		await expect.element(dialog).toBeVisible();
		await expect
			.element(dialog.getByText(/« Audio » n’est placé nulle part ailleurs/))
			.toBeVisible();
		expect(onassign).not.toHaveBeenCalled();
		await page.getByRole('button', { name: 'Annuler' }).click();
		await expect.element(page.getByRole('dialog')).not.toBeInTheDocument();
		await expect.element(select).toHaveValue('none');
		expect(onassign).not.toHaveBeenCalled();
		await page.getByRole('button', { name: 'Affecter' }).click();
		await page.getByRole('button', { name: 'Retirer' }).click();
		await expect.element(page.getByRole('dialog')).not.toBeInTheDocument();
		expect(onassign).toHaveBeenCalledOnce();
		expect(onassign).toHaveBeenCalledWith({ type: 'none' });
	});

	it('asks to confirm when a function of the manager replaces the module', async () => {
		const { onassign } = await show();
		await page.getByRole('combobox', { name: 'Fonction' }).selectOptions('Navigation');
		await page.getByRole('button', { name: 'Affecter' }).click();
		await expect.element(page.getByRole('dialog')).toBeVisible();
		expect(onassign).not.toHaveBeenCalled();
	});

	it('does not ask when the module stays placed elsewhere', async () => {
		const config = { ...CONFIG, home: ['audio'] } as unknown as DeviceConfig;
		const { onassign } = await show(VOLUME, config);
		await page.getByRole('combobox', { name: 'Fonction' }).selectOptions('Rien');
		await page.getByRole('button', { name: 'Affecter' }).click();
		await expect.element(page.getByRole('dialog')).not.toBeInTheDocument();
		expect(onassign).toHaveBeenCalledWith({ type: 'none' });
	});

	it('says what the function being chosen does', async () => {
		await show({ kind: 'navigation' });
		await expect.element(page.getByText(/fait défiler les lignes de la page/)).toBeVisible();
		await page.getByRole('combobox', { name: 'Fonction' }).selectOptions('Fonction contextuelle');
		await expect.element(page.getByText(/Dépend de la page affichée/)).toBeVisible();
		await page.getByRole('combobox', { name: 'Fonction' }).selectOptions('Contrôle du volume');
		await expect.element(page.getByText(/Réglage de Audio\./)).toBeVisible();
	});

	it('warns that a module placed nowhere else is deleted with its settings', async () => {
		await show();
		await page.getByRole('combobox', { name: 'Fonction' }).selectOptions('Rien');
		await expect.element(page.getByText(/il sera supprimé/)).toBeVisible();
	});

	it('has no settings to edit for a function of the manager', async () => {
		await show({ kind: 'navigation' });
		await expect
			.element(page.getByRole('combobox', { name: 'Fonction' }))
			.toHaveValue('navigation');
		await expect.element(page.getByLabelText('Libellé')).not.toBeInTheDocument();
	});

	it('edits the settings of the module of the function, without the label of a key', async () => {
		await show();
		await expect.element(page.getByText(/Réglages de « Audio »/)).toBeVisible();
		await expect.element(page.getByLabelText('Libellé')).not.toBeInTheDocument();
	});
});
