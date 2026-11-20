import { describe, expect, it } from 'vitest';
import type { MouseConfig, RadialMenuConfig } from '#lib/generated/index.js';
import {
	addEntry,
	bindControl,
	hapticKey,
	isShownCountValid,
	moveEntry,
	removeEntry,
	setDpi,
	setEntryEnabled,
	setEntryPrimary,
	setHapticEvent,
	setHapticLevel,
	setHoldMs,
	setRadialOptions,
	setThumbWheel,
	setWheel,
	shownCount
} from './mouse-edits.js';

const MOUSE: MouseConfig = {
	schemaVersion: 1,
	serial: 'mx',
	model: 'mxMaster4',
	bindings: { back: { kind: 'doubleClick' }, gesture: { kind: 'radialMenu' } },
	pointer: { dpi: 1600 },
	wheel: { mode: 'ratchet', autoThreshold: 20, natural: false, hiRes: true },
	thumbWheel: { speed: 5, invert: false },
	haptics: { level: 'medium', events: {} },
	revision: 3
};

const entry = (module: string, enabled = true) => ({
	module,
	enabled,
	primary: { kind: 'action' as const, action: 'toggle' }
});

const MENU: RadialMenuConfig = {
	schemaVersion: 1,
	holdMs: 280,
	animate: true,
	haptics: true,
	entries: [entry('a'), entry('b'), entry('c', false)],
	revision: 1
};

describe('bindControl', () => {
	it('binds a control without touching the others', () => {
		const next = bindControl('forward', { kind: 'click', button: 'middle' })(MOUSE);
		expect(next.bindings.forward).toEqual({ kind: 'click', button: 'middle' });
		expect(next.bindings.back).toEqual({ kind: 'doubleClick' });
		expect(MOUSE.bindings.forward).toBeUndefined();
	});

	it('gives a control back to the mouse', () => {
		expect(bindControl('back', null)(MOUSE).bindings.back).toBeUndefined();
	});

	it('moves the menu to the control that gets it', () => {
		const next = bindControl('wheelMode', { kind: 'radialMenu' })(MOUSE);
		expect(next.bindings.wheelMode).toEqual({ kind: 'radialMenu' });
		expect(next.bindings.gesture).toBeUndefined();
	});

	it('keeps the menu where it is when another action is bound elsewhere', () => {
		expect(bindControl('back', { kind: 'none' })(MOUSE).bindings.gesture).toEqual({
			kind: 'radialMenu'
		});
	});
});

describe('settings edits', () => {
	it('changes the pointer, the wheels and the haptic level', () => {
		expect(setDpi(800)(MOUSE).pointer.dpi).toBe(800);
		expect(setWheel({ natural: true })(MOUSE).wheel).toEqual({ ...MOUSE.wheel, natural: true });
		expect(setThumbWheel({ speed: 9 })(MOUSE).thumbWheel).toEqual({ speed: 9, invert: false });
		expect(setHapticLevel('high')(MOUSE).haptics.level).toBe('high');
	});

	it('sets and clears the play of a haptic event', () => {
		const key = hapticKey('pomodoro', 'finished');
		expect(key).toBe('pomodoro.finished');
		const set = setHapticEvent(key, { enabled: false, waveform: 'completed' })(MOUSE);
		expect(set.haptics.events[key]).toEqual({ enabled: false, waveform: 'completed' });
		expect(setHapticEvent(key, null)(set).haptics.events).toEqual({});
	});
});

describe('radial menu edits', () => {
	it('changes the hold time and the options', () => {
		expect(setHoldMs(400)(MENU).holdMs).toBe(400);
		expect(setRadialOptions({ animate: false })(MENU).animate).toBe(false);
		expect(setRadialOptions({ haptics: false })(MENU).haptics).toBe(false);
	});

	it('changes one entry', () => {
		expect(setEntryEnabled('c', true)(MENU).entries[2].enabled).toBe(true);
		expect(setEntryPrimary('a', { kind: 'close' })(MENU).entries[0].primary).toEqual({
			kind: 'close'
		});
		expect(setEntryEnabled('missing', true)(MENU)).toEqual(MENU);
	});

	it('moves an entry within the list', () => {
		expect(moveEntry('b', -1)(MENU).entries.map((e) => e.module)).toEqual(['b', 'a', 'c']);
		expect(moveEntry('a', 1)(MENU).entries.map((e) => e.module)).toEqual(['b', 'a', 'c']);
		expect(moveEntry('a', -1)(MENU)).toBe(MENU);
		expect(moveEntry('c', 5)(MENU)).toBe(MENU);
		expect(moveEntry('missing', 1)(MENU)).toBe(MENU);
	});

	it('adds an entry once and removes it', () => {
		const added = addEntry('d', 'open')(MENU);
		expect(added.entries[3]).toEqual({
			module: 'd',
			enabled: true,
			primary: { kind: 'action', action: 'open' }
		});
		expect(addEntry('d', 'open')(added)).toBe(added);
		expect(removeEntry('d')(added).entries).toHaveLength(3);
	});

	it('counts the entries shown against what the manager accepts', () => {
		expect(shownCount(MENU)).toBe(2);
		expect(isShownCountValid(MENU)).toBe(false);
		expect(isShownCountValid(setEntryEnabled('c', true)(MENU))).toBe(true);
	});
});
