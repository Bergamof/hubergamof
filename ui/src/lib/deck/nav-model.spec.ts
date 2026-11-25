import { describe, expect, it } from 'vitest';
import type { DeviceInfo, HeadsetInfo, MouseInfo } from '#lib/generated/index.js';
import { PATHS, deviceQuery, navDevices, pageOf } from './nav-model.js';

const deck = { serial: 'A1', displayName: 'Stream Deck +' } as DeviceInfo;
const mouse = { serial: 'M 1', displayName: 'MX Master 4' } as MouseInfo;
const headset = { address: '38:18', name: 'WH-1000XM4' } as HeadsetInfo;

describe('pageOf', () => {
	it('finds a page by its address, with or without a trailing slash', () => {
		expect(pageOf(PATHS.ssh).title).toBe('SSH');
		expect(pageOf('/poste/ssh/').title).toBe('SSH');
		expect(pageOf(PATHS.ssh).group).toBe('Poste');
	});

	it('falls back to the home page for an unknown address', () => {
		expect(pageOf('/nulle-part').title).toBe('Accueil');
		expect(pageOf('/').group).toBeUndefined();
	});
});

describe('navDevices', () => {
	it('lists the Stream Deck, then the mice, then the headphones', () => {
		const devices = navDevices([deck], [mouse], [headset]);
		expect(devices.map((d) => d.kind)).toEqual(['deck', 'mouse', 'headset']);
		expect(devices.map((d) => d.label)).toEqual(['Stream Deck +', 'MX Master 4', 'WH-1000XM4']);
		expect(devices[1]).toMatchObject({ path: PATHS.mouse, query: '?appareil=M%201' });
	});

	it('lists nothing when nothing is connected', () => {
		expect(navDevices([], [], [])).toEqual([]);
	});

	it('tells devices of one kind apart by an encoded query', () => {
		expect(deviceQuery('mouse', 'M 1')).toBe('?appareil=M%201');
		expect(deviceQuery('deck', 'A1')).toBe('?appareil=A1');
		expect(deviceQuery('headset', '38:18')).toBe('');
	});
});
