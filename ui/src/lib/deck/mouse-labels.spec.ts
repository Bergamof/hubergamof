import { describe, expect, it } from 'vitest';
import { CATALOG } from '../components/mouse-fixtures.js';
import { describeAction, settingLabel } from './mouse-labels.js';

describe('settingLabel', () => {
	it('names the settings the manager reports', () => {
		expect(settingLabel('sensitivity')).toBe('la sensibilité du pointeur');
		expect(settingLabel('button')).toBe('l’action d’un bouton');
	});

	it('shows an unknown setting as it is', () => {
		expect(settingLabel('something new')).toBe('something new');
	});
});

describe('describeAction', () => {
	const describe_ = (action: Parameters<typeof describeAction>[0]) =>
		describeAction(action, 'Précédent', CATALOG);

	it('says what the mouse does by itself when the control has no action', () => {
		expect(describe_(undefined)).toBe('Précédent');
	});

	it('says what each action does', () => {
		expect(describe_({ kind: 'none' })).toBe('Désactivé');
		expect(describe_({ kind: 'click', button: 'middle' })).toBe('Clic milieu');
		expect(describe_({ kind: 'doubleClick' })).toBe('Double clic');
		expect(describe_({ kind: 'toggleWheelMode' })).toBe('Cran / libre');
		expect(describe_({ kind: 'radialMenu' })).toBe('Menu circulaire');
		expect(describe_({ kind: 'shortcut', keys: 'ctrl+shift+m' })).toBe('Raccourci Ctrl + Maj + M');
	});

	it('names the module and its action, by their identifiers when the catalog does not know them', () => {
		expect(describe_({ kind: 'module', module: 'pomodoro', action: 'toggle' })).toBe(
			'Pomodoro › Démarrer ou mettre en pause'
		);
		expect(describe_({ kind: 'module', module: 'gone', action: 'old' })).toBe('gone › old');
		expect(describe_({ kind: 'module', module: 'pomodoro', action: 'old' })).toBe('Pomodoro › old');
	});
});
