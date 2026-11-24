import { describe, expect, it } from 'vitest';
import { presetOptions } from './headset-labels.js';

describe('presetOptions', () => {
	it('lists the presets to choose from, without the manual one and the unknown one', () => {
		const values = presetOptions('speech').map((option) => option.value);
		expect(values).toContain('bassBoost');
		expect(values).not.toContain('manual');
		expect(values).not.toContain('other');
	});

	it('adds the preset in use when it cannot be chosen, first', () => {
		expect(presetOptions('manual')[0]).toEqual({ value: 'manual', label: 'Manuel' });
		expect(presetOptions('other')[0]).toEqual({ value: 'other', label: 'Autre' });
	});
});
