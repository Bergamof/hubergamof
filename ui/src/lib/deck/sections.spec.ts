import { describe, expect, it } from 'vitest';
import { availableSections, sectionShown } from './sections.js';

describe('availableSections', () => {
	it('always offers the radial menu, with the kinds of device that are connected', () => {
		expect(availableSections(true, true)).toEqual(['deck', 'mouse', 'menu']);
		expect(availableSections(true, false)).toEqual(['deck', 'menu']);
		expect(availableSections(false, true)).toEqual(['mouse', 'menu']);
		expect(availableSections(false, false)).toEqual(['menu']);
	});

	it('offers the modules of the application, last, when the manager has some', () => {
		expect(availableSections(false, false, true)).toEqual(['menu', 'modules']);
		expect(availableSections(true, true, true)).toEqual(['deck', 'mouse', 'menu', 'modules']);
	});
});

describe('sectionShown', () => {
	it('keeps what was asked for while it is available, else the first section', () => {
		expect(sectionShown('menu', ['deck', 'menu'])).toBe('menu');
		expect(sectionShown('deck', ['mouse', 'menu'])).toBe('mouse');
		expect(sectionShown('mouse', ['menu'])).toBe('menu');
	});
});
