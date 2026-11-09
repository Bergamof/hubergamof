import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
import KeyGrid from './KeyGrid.svelte';

const CLOCK: CatalogModule = {
	icon: null,
	settingIcons: {},
	defaultSettings: {},
	kind: 'clock',
	name: 'Horloge',
	description: '',
	capabilities: { key: true, dial: false, touchZone: false },
	settings: []
};

function instance(id: string, kind: string, label: string | null = null): ModuleInstance {
	return { id, kind, label, image: null, settings: {} };
}

async function renderGrid(keys: readonly (ModuleInstance | undefined)[], selected?: number) {
	const handlers = { onselect: vi.fn(), onmove: vi.fn(), onadd: vi.fn() };
	await render(KeyGrid, {
		keys,
		moduleOf: (kind: string) => (kind === CLOCK.kind ? CLOCK : undefined),
		imageOf: () => null,
		columns: 2,
		visibleRows: 1,
		selected,
		buttonId: (position: number) => `key-${position}`,
		...handlers
	});
	return handlers;
}

/** Key button at `position` (from 1), its accessible name starting with the module's. */
const key = (position: number, name: string) =>
	page.getByRole('button', { name: new RegExp(`^Touche ${position} : ${name}`) });

describe('KeyGrid', () => {
	it('shows each key by name, in order, the selected one current', async () => {
		await renderGrid([instance('a', 'clock'), instance('b', 'clock', 'Paris')], 1);
		await expect.element(key(1, 'Horloge')).not.toHaveAttribute('aria-current');
		await expect.element(key(2, 'Paris')).toHaveAttribute('aria-current', 'true');
		await expect.element(key(2, 'Paris')).toHaveAttribute('id', 'key-1');
		await expect
			.element(page.getByRole('button', { name: 'Ajouter une touche' }))
			.toHaveAttribute('id', 'key--1');
	});

	it('marks keys whose module is missing or unknown', async () => {
		await renderGrid([undefined, instance('c', 'gone')]);
		// Said in their accessible name.
		await expect.element(key(1, 'Module manquant indisponible')).toBeVisible();
		await expect.element(key(2, 'gone indisponible')).toBeVisible();
	});

	it('selects, adds and reorders keys', async () => {
		const { onselect, onmove, onadd } = await renderGrid([
			instance('a', 'clock', 'Un'),
			instance('b', 'clock', 'Deux')
		]);
		await key(2, 'Deux').click();
		expect(onselect).toHaveBeenCalledExactlyOnceWith(1);
		await page.getByRole('button', { name: 'Ajouter une touche' }).click();
		expect(onadd).toHaveBeenCalledOnce();
		await key(1, 'Un').dropTo(key(2, 'Deux'));
		expect(onmove).toHaveBeenCalledExactlyOnceWith(0, 1);
	});

	it('explains scrolling once the keys exceed what the device shows', async () => {
		const scrolling = page.getByText(/faisant défiler/);
		await renderGrid([instance('a', 'clock')]);
		await expect.element(scrolling).not.toBeInTheDocument();
		await renderGrid([instance('a', 'clock'), instance('b', 'clock')]);
		await expect.element(scrolling).toBeVisible();
	});
});
