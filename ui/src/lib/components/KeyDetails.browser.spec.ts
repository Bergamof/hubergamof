import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ManagerApi } from '#lib/api/manager-api.js';
import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
import KeyDetails from './KeyDetails.svelte';

const AUDIO: CatalogModule = {
	kind: 'audio',
	name: 'Audio',
	description: '',
	capabilities: { key: true, multipleInstances: false },
	dialFunctions: [],
	icon: null,
	settingIcons: {},
	defaultSettings: {},
	settings: []
};

const INSTANCE: ModuleInstance = {
	id: 'audio',
	kind: 'audio',
	label: null,
	image: null,
	settings: {}
};

async function show(sharedWithDial: boolean) {
	await render(KeyDetails, {
		position: 0,
		count: 1,
		instance: INSTANCE,
		module: AUDIO,
		sharedWithDial,
		serial: 'DECK',
		api: {} as ManagerApi,
		headingId: 'details-heading',
		onmove: vi.fn(),
		onremove: vi.fn(),
		onapply: vi.fn(),
		ondiscard: vi.fn(),
		onimage: vi.fn()
	});
}

describe('KeyDetails', () => {
	it('tells that the settings are shared with the dial of a module that exists once', async () => {
		await show(true);
		await expect.element(page.getByText(/réglages sont partagés avec sa molette/)).toBeVisible();
	});

	it('says nothing about sharing otherwise', async () => {
		await show(false);
		await expect
			.element(page.getByText(/réglages sont partagés avec sa molette/))
			.not.toBeInTheDocument();
	});
});
