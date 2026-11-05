import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { DeviceInfo } from '#lib/generated/index.js';
import DeviceSelector from './DeviceSelector.svelte';

function device(serial: string, displayName: string): DeviceInfo {
	return {
		serial,
		model: 'virtual',
		displayName,
		capabilities: {
			keyColumns: 4,
			keyRows: 2,
			keyImageSize: { width: 120, height: 120 },
			dialCount: 4,
			touchStrip: null
		}
	};
}

const DESK = device('DESK', 'Stream Deck+ (bureau)');
const STUDIO = device('STUDIO', 'Stream Deck+ (studio)');

describe('DeviceSelector', () => {
	it('names a single device without a choice', async () => {
		await render(DeviceSelector, { devices: [DESK], selected: 'DESK', onselect: vi.fn() });
		await expect.element(page.getByRole('heading', { name: DESK.displayName })).toBeVisible();
		await expect.element(page.getByRole('combobox')).not.toBeInTheDocument();
	});

	it('lets the user pick among several devices', async () => {
		const onselect = vi.fn();
		await render(DeviceSelector, { devices: [DESK, STUDIO], selected: 'DESK', onselect });
		const choice = page.getByRole('combobox', { name: 'Appareil' });
		await expect.element(choice).toHaveValue('DESK');
		await choice.selectOptions(STUDIO.displayName);
		expect(onselect).toHaveBeenCalledExactlyOnceWith('STUDIO');
	});
});
