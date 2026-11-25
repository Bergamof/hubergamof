import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { navDevices, type NavDevice } from '#lib/deck/nav-model.js';
import type { DeviceInfo, HeadsetInfo, MouseInfo } from '#lib/generated/index.js';
import SideNav from './SideNav.svelte';

const DECK = { serial: 'A1', displayName: 'Stream Deck +' } as DeviceInfo;
const MOUSE = { serial: 'M1', displayName: 'MX Master 4' } as MouseInfo;
const OTHER_MOUSE = { serial: 'M2', displayName: 'MX Anywhere 3S' } as MouseInfo;
const HEADSET = { address: '38:18', name: 'WH-1000XM4' } as HeadsetInfo;

function props(overrides: Partial<Parameters<typeof SideNav>[1]> = {}) {
	return {
		pathname: '/',
		devices: [] as NavDevice[],
		selectedDeck: undefined,
		selectedMouse: undefined,
		mouseBattery: undefined,
		updateAvailable: false,
		...overrides
	};
}

describe('SideNav', () => {
	it('names the groups and the pages of the machine and of the application', async () => {
		await render(SideNav, props());
		for (const name of ['Périphériques', 'Poste', 'Application']) {
			await expect.element(page.getByRole('heading', { name, level: 2 })).toBeVisible();
		}
		for (const name of [
			'Alias du shell',
			'Git',
			'SSH',
			'Modules',
			'Menu circulaire',
			'Sauvegarde'
		]) {
			await expect.element(page.getByRole('link', { name })).toBeVisible();
		}
	});

	it('marks the page shown with aria-current', async () => {
		await render(SideNav, props({ pathname: '/poste/ssh' }));
		await expect
			.element(page.getByRole('link', { name: 'SSH' }))
			.toHaveAttribute('aria-current', 'page');
		await expect
			.element(page.getByRole('link', { name: 'Git' }))
			.not.toHaveAttribute('aria-current');
	});

	it('says no device is connected, keeping the group', async () => {
		await render(SideNav, props());
		await expect.element(page.getByRole('link', { name: 'Aucun connecté' })).toBeVisible();
	});

	it('lists the connected devices and marks the one the page follows', async () => {
		await render(
			SideNav,
			props({
				pathname: '/peripheriques/souris',
				devices: navDevices([DECK], [MOUSE, OTHER_MOUSE], [HEADSET]),
				selectedDeck: 'A1',
				selectedMouse: 'M2'
			})
		);
		await expect.element(page.getByRole('link', { name: 'Stream Deck +' })).toBeVisible();
		await expect.element(page.getByRole('link', { name: 'WH-1000XM4' })).toBeVisible();
		await expect
			.element(page.getByRole('link', { name: 'MX Anywhere 3S' }))
			.toHaveAttribute('aria-current', 'page');
		await expect
			.element(page.getByRole('link', { name: 'MX Master 4' }))
			.not.toHaveAttribute('aria-current');
	});

	it('shows the battery of the mouse followed', async () => {
		await render(
			SideNav,
			props({
				pathname: '/peripheriques/souris',
				devices: navDevices([], [MOUSE], []),
				selectedMouse: 'M1',
				mouseBattery: 78
			})
		);
		await expect.element(page.getByText(/^78 %$/)).toBeVisible();
	});

	it('points to an update that is waiting', async () => {
		await render(SideNav, props({ updateAvailable: true }));
		await expect.element(page.getByText(/^Disponible$/)).toBeVisible();
	});
});
