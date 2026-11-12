import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { CatalogModule } from '#lib/generated/index.js';
import ModulePicker from './ModulePicker.svelte';

function module(kind: string, name: string): CatalogModule {
	return {
		kind,
		name,
		description: `Description de ${name}.`,
		capabilities: { key: true, multipleInstances: false },
		dialFunctions: [],
		icon: null,
		settingIcons: {},
		defaultSettings: {},
		settings: []
	};
}

const MODULES = [module('audio', 'Audio'), module('claude', 'Claude')];

describe('ModulePicker', () => {
	it('adds what the notice says about a module under its description', async () => {
		await render(ModulePicker, {
			open: true,
			title: 'Ajouter une touche',
			modules: MODULES,
			notice: (candidate: CatalogModule) =>
				candidate.kind === 'audio' ? 'Réglages partagés avec la molette.' : undefined,
			onpick: vi.fn(),
			onclose: vi.fn()
		});
		await expect.element(page.getByText('Réglages partagés avec la molette.')).toBeVisible();
		await expect.element(page.getByText('Description de Claude.')).toBeVisible();
		expect(document.querySelectorAll('dialog li')).toHaveLength(2);
	});

	it('says why there is nothing to pick', async () => {
		await render(ModulePicker, {
			open: true,
			title: 'Ajouter une touche',
			modules: [],
			empty: 'Tout est déjà placé.',
			onpick: vi.fn(),
			onclose: vi.fn()
		});
		await expect.element(page.getByText('Tout est déjà placé.')).toBeVisible();
	});

	it('picks the module clicked', async () => {
		const onpick = vi.fn();
		await render(ModulePicker, {
			open: true,
			title: 'Ajouter une touche',
			modules: MODULES,
			onpick,
			onclose: vi.fn()
		});
		await page.getByRole('button', { name: /Claude/ }).click();
		expect(onpick).toHaveBeenCalledWith(MODULES[1]);
	});
});
