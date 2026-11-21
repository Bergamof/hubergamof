import { describe, expect, it } from 'vitest';
import { settingLabel } from './mouse-labels.js';

describe('settingLabel', () => {
	it('names the settings the manager reports', () => {
		expect(settingLabel('sensitivity')).toBe('la sensibilité du pointeur');
		expect(settingLabel('button')).toBe('l’action d’un bouton');
	});

	it('shows an unknown setting as it is', () => {
		expect(settingLabel('something new')).toBe('something new');
	});
});
