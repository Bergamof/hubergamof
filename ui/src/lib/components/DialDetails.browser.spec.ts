import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ManagerApi } from '#lib/api/manager-api.js';
import type { CatalogModule, DeviceConfig, ModuleInstance } from '#lib/generated/index.js';
import DialDetails from './DialDetails.svelte';

function module(kind: string, name: string): CatalogModule {
	return {
		kind,
		name,
		description: '',
		capabilities: { key: true, dial: true, touchZone: true },
		icon: null,
		settingIcons: {},
		defaultSettings: {},
		settings: []
	};
}

const MODULES = [module('audio', 'Audio'), module('clock', 'Horloge')];

function instance(kind: string): ModuleInstance {
	return { id: kind, kind, label: null, image: null, settings: {} };
}

const CONFIG = {
	serial: 'DECK',
	home: [],
	dials: ['audio'],
	modules: [instance('audio')]
} as unknown as DeviceConfig;

async function show(current: ModuleInstance | null = instance('audio')) {
	const onassign = vi.fn();
	await render(DialDetails, {
		index: 0,
		config: CONFIG,
		serial: 'DECK',
		api: {} as ManagerApi,
		instance: current,
		candidates: [{ instance: instance('audio'), place: 'molette 1' }],
		modules: MODULES,
		moduleOf: (kind: string) => MODULES.find((candidate) => candidate.kind === kind),
		headingId: 'details-heading',
		onassign,
		onapply: vi.fn(),
		ondiscard: vi.fn()
	});
	return { onassign };
}

describe('DialDetails', () => {
	it('names the dial and offers the modules, the current one selected', async () => {
		await show();
		await expect.element(page.getByRole('heading', { name: 'Molette 1' })).toBeVisible();
		await expect
			.element(page.getByRole('combobox', { name: 'Module' }))
			.toHaveValue('instance:audio');
		await expect
			.element(page.getByRole('option', { name: 'Horloge (nouveau)' }))
			.toBeInTheDocument();
	});

	it('assigns a module only when the button is pressed, never by browsing the list', async () => {
		const { onassign } = await show();
		const assign = page.getByRole('button', { name: 'Affecter' });
		await expect.element(assign).toHaveAttribute('aria-disabled', 'true');
		await page.getByRole('combobox', { name: 'Module' }).selectOptions('Horloge (nouveau)');
		expect(onassign).not.toHaveBeenCalled();
		await expect.element(assign).toHaveAttribute('aria-disabled', 'false');
		await assign.click();
		expect(onassign).toHaveBeenCalledOnce();
		expect(onassign.mock.calls[0]?.[0]).toMatchObject({ type: 'new', module: { kind: 'clock' } });
	});

	it('warns that a module placed nowhere else is deleted with its settings', async () => {
		await show();
		await page.getByRole('combobox', { name: 'Module' }).selectOptions('Aucun');
		await expect.element(page.getByText(/« Audio » n’est placé nulle part ailleurs/)).toBeVisible();
	});

	it('has no settings to edit for a dial without module', async () => {
		await show(null);
		await expect.element(page.getByRole('combobox', { name: 'Module' })).toHaveValue('none');
		await expect.element(page.getByLabelText('Libellé')).not.toBeInTheDocument();
	});
});
