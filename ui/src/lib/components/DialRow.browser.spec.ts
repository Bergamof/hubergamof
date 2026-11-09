import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
import DialRow from './DialRow.svelte';

const AUDIO: CatalogModule = {
	icon: 'data:image/png;base64,AAAA',
	settingIcons: {},
	defaultSettings: {},
	kind: 'audio',
	name: 'Audio',
	description: '',
	capabilities: { key: true, dial: true, touchZone: true },
	settings: []
};

function instance(kind: string, label: string | null = null): ModuleInstance {
	return { id: kind, kind, label, image: null, settings: {} };
}

async function renderRow(dials: readonly (ModuleInstance | null)[], selected?: number) {
	const onselect = vi.fn();
	const { container } = await render(DialRow, {
		dials,
		moduleOf: (kind: string) => (kind === AUDIO.kind ? AUDIO : undefined),
		selected,
		onselect
	});
	return { container, onselect };
}

describe('DialRow', () => {
	it('names each dial by its module, a free one as such', async () => {
		await renderRow([instance('audio'), null]);
		await expect.element(page.getByRole('button', { name: 'Molette 1 : Audio' })).toBeVisible();
		await expect
			.element(page.getByRole('button', { name: 'Molette 2 : Aucun module' }))
			.toBeVisible();
	});

	it('draws a knob, not the module icon already shown on the strip', async () => {
		const { container } = await renderRow([instance('audio'), null]);
		expect(container.querySelectorAll('img')).toHaveLength(0);
		const knobs = [...container.querySelectorAll('svg[data-assigned]')];
		expect(knobs.map((knob) => knob.getAttribute('data-assigned'))).toEqual(['true', 'false']);
	});

	it('marks the selected dial current and reports a click', async () => {
		const { onselect } = await renderRow([instance('audio'), instance('audio', 'Casque')], 1);
		const second = page.getByRole('button', { name: 'Molette 2 : Casque' });
		await expect.element(second).toHaveAttribute('aria-current', 'true');
		await page.getByRole('button', { name: 'Molette 1 : Audio' }).click();
		expect(onselect).toHaveBeenCalledWith(0);
	});

	it('flags a module this manager does not provide', async () => {
		await renderRow([instance('ghost')]);
		await expect.element(page.getByText('indisponible')).toBeVisible();
	});
});
