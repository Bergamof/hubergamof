import { describe, expect, it } from 'vitest';
import {
	isReserved,
	keyName,
	modifierName,
	normalizeShortcut,
	shortcutLabel,
	shortcutOf
} from './shortcut.js';

describe('keyName', () => {
	it.each([
		['KeyM', 'm'],
		['Digit7', '7'],
		['F1', 'f1'],
		['F12', 'f12'],
		['ControlRight', 'ctrl'],
		['MetaLeft', 'super'],
		['Escape', 'esc'],
		['Period', 'dot'],
		['ArrowLeft', 'left'],
		['MediaPlayPause', 'playpause'],
		['AudioVolumeMute', 'mute']
	])('names %s %s', (code, name) => {
		expect(keyName(code)).toBe(name);
	});

	it.each(['F13', 'F0', 'F01', 'Numpad5', 'CapsLock', 'IntlBackslash', ''])(
		'cannot press %s',
		(code) => {
			expect(keyName(code)).toBeUndefined();
		}
	);
});

describe('modifierName', () => {
	it('tells modifiers from other keys', () => {
		expect(modifierName('ShiftLeft')).toBe('shift');
		expect(modifierName('KeyA')).toBeUndefined();
	});
});

describe('shortcutOf', () => {
	it('puts the modifiers first and drops duplicates', () => {
		expect(shortcutOf(['KeyM', 'ShiftLeft', 'ControlLeft', 'ControlRight'])).toBe('ctrl+shift+m');
	});

	it('refuses nothing, an unknown key and too many keys', () => {
		expect(shortcutOf([])).toBeUndefined();
		expect(shortcutOf(['KeyA', 'CapsLock'])).toBeUndefined();
		expect(shortcutOf(['KeyA', 'KeyB', 'KeyC', 'KeyD', 'KeyE', 'KeyF', 'KeyG'])).toBeUndefined();
	});
});

describe('isReserved', () => {
	it.each(['ctrl+alt+delete', 'alt+ctrl+backspace', 'ctrl+alt+f3'])('%s is reserved', (keys) => {
		expect(isReserved(keys)).toBe(true);
	});

	it.each(['ctrl+delete', 'ctrl+alt+t', 'alt+f4'])('%s is allowed', (keys) => {
		expect(isReserved(keys)).toBe(false);
	});
});

describe('shortcutLabel', () => {
	it('writes the keys the way the person reads them', () => {
		expect(shortcutLabel('ctrl+shift+m')).toBe('Ctrl + Maj + M');
		expect(shortcutLabel('super+left')).toBe('Super + ←');
	});
});

describe('normalizeShortcut', () => {
	it('reads what a person types', () => {
		expect(normalizeShortcut('Ctrl + Maj + M')).toBe('ctrl+shift+m');
		expect(normalizeShortcut('control+win+return')).toBe('ctrl+super+enter');
		expect(normalizeShortcut('F5')).toBe('f5');
	});

	it('refuses unknown keys, nothing and too many keys', () => {
		expect(normalizeShortcut('ctrl+banane')).toBeUndefined();
		expect(normalizeShortcut('')).toBeUndefined();
		expect(normalizeShortcut('ctrl++m')).toBeUndefined();
		expect(normalizeShortcut('a+b+c+d+e+f+g')).toBeUndefined();
		expect(normalizeShortcut('f13')).toBeUndefined();
	});
});
