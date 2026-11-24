import { describe, expect, it } from 'vitest';
import { availableSections, sectionShown } from './sections.js';

describe('availableSections', () => {
	it('always offers the radial menu and the machine, with the kinds of device that are connected', () => {
		expect(availableSections(true, true)).toEqual(['deck', 'mouse', 'menu', 'machine']);
		expect(availableSections(true, false)).toEqual(['deck', 'menu', 'machine']);
		expect(availableSections(false, true)).toEqual(['mouse', 'menu', 'machine']);
		expect(availableSections(false, false)).toEqual(['menu', 'machine']);
	});

	it('offers the modules of the application before the machine when the manager has some', () => {
		expect(availableSections(false, false, true)).toEqual(['menu', 'modules', 'machine']);
		expect(availableSections(true, true, true)).toEqual([
			'deck',
			'mouse',
			'menu',
			'modules',
			'machine'
		]);
	});
});

describe('sectionShown', () => {
	it('keeps what was asked for while it is available, else the first section', () => {
		expect(sectionShown('menu', ['deck', 'menu'])).toBe('menu');
		expect(sectionShown('deck', ['mouse', 'menu'])).toBe('mouse');
		expect(sectionShown('mouse', ['menu'])).toBe('menu');
		expect(sectionShown('machine', ['menu', 'machine'])).toBe('machine');
	});
});
