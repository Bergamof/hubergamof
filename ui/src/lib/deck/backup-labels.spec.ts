import { describe, expect, it } from 'vitest';
import {
	CHANGE_LABELS,
	deviceSerialOf,
	outcomeText,
	sectionLabel,
	snapshotDate
} from './backup-labels.js';

describe('sectionLabel', () => {
	it('names the sections that are not tied to a device', () => {
		expect(sectionLabel('radial-menu')).toBe('Menu circulaire');
		expect(sectionLabel('modules')).toBe('Modules');
		expect(sectionLabel('machine')).toBe('Alias du shell');
		expect(sectionLabel('git')).toBe('Git');
		expect(sectionLabel('ssh')).toBe('SSH');
	});

	it('names a device or a mouse by its serial, and by its name when it is connected', () => {
		expect(sectionLabel('devices/AB12')).toBe('Stream Deck AB12');
		expect(sectionLabel('mice/MX1')).toBe('Souris MX1');
		const names = new Map([
			['AB12', 'Stream Deck +'],
			['MX1', 'MX Master 4']
		]);
		expect(sectionLabel('devices/AB12', names)).toBe('Stream Deck + (AB12)');
		expect(sectionLabel('mice/MX1', names)).toBe('MX Master 4 (MX1)');
	});

	it('leaves what it does not know as it is', () => {
		expect(sectionLabel('other/1')).toBe('other/1');
	});
});

describe('deviceSerialOf', () => {
	it('reads the serial of a device section only', () => {
		expect(deviceSerialOf('devices/AB12')).toBe('AB12');
		expect(deviceSerialOf('mice/MX1')).toBeUndefined();
		expect(deviceSerialOf('git')).toBeUndefined();
	});
});

describe('outcomeText', () => {
	it('says in words what became of a section', () => {
		expect(outcomeText({ result: 'applied' }).label).toBe('Appliquée');
		expect(outcomeText({ result: 'stored' }).label).toMatch(/prochain branchement/);
		expect(outcomeText({ result: 'notApplied', message: 'no home' })).toEqual({
			label: 'Enregistrée, mais pas appliquée à l’ordinateur',
			detail: 'no home'
		});
		expect(outcomeText({ result: 'refused', code: 'invalidConfig', message: 'bad' })).toEqual({
			label: 'Refusée, laissée telle quelle',
			detail: 'bad'
		});
	});
});

describe('labels', () => {
	it('words every change', () => {
		expect(CHANGE_LABELS).toEqual({ added: 'Ajoutée', replaced: 'Remplacée', same: 'Identique' });
	});

	it('shows the date of a snapshot in French', () => {
		expect(snapshotDate(Date.UTC(2026, 10, 25, 12, 0) / 1000)).toMatch(/novembre 2026/);
	});
});
