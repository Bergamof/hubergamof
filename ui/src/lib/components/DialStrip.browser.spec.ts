import { describe, expect, it } from 'vitest';
import { render } from 'vitest-browser-svelte';
import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
import DialStrip from './DialStrip.svelte';

const ICON = 'data:image/png;base64,AAAA';

const AUDIO: CatalogModule = {
	icon: ICON,
	settingIcons: {},
	defaultSettings: {},
	kind: 'audio',
	name: 'Audio',
	description: '',
	capabilities: { key: true, dial: true, touchZone: true },
	settings: []
};

const NO_ICON: CatalogModule = { ...AUDIO, kind: 'plain', icon: null };

function instance(kind: string): ModuleInstance {
	return { id: kind, kind, label: null, image: null, settings: {} };
}

const MODULES = new Map([AUDIO, NO_ICON].map((module) => [module.kind, module]));

async function renderStrip(dials: readonly (ModuleInstance | null)[]) {
	const { container } = await render(DialStrip, {
		dials,
		moduleOf: (kind: string) => MODULES.get(kind)
	});
	return container;
}

describe('DialStrip', () => {
	it('shows one zone per dial, with the icon of its module', async () => {
		const container = await renderStrip([instance('audio'), instance('audio'), null]);
		expect(container.querySelectorAll('li')).toHaveLength(3);
		expect([...container.querySelectorAll('img')].map((image) => image.src)).toEqual([ICON, ICON]);
	});

	it('draws a dash for a free dial and for a module without icon', async () => {
		const container = await renderStrip([null, instance('plain'), instance('missing')]);
		expect(container.querySelectorAll('img')).toHaveLength(0);
		expect(container.querySelectorAll('li > span')).toHaveLength(3);
	});

	it('is hidden from assistive technology: the dials underneath carry the names', async () => {
		const container = await renderStrip([instance('audio')]);
		expect(container.querySelector('ol')?.getAttribute('aria-hidden')).toBe('true');
	});
});
