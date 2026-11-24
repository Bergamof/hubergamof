import { describe, expect, it } from 'vitest';
import type { AdvancedSetting } from '#lib/generated/index.js';
import {
	addSetting,
	asAlias,
	fromAlias,
	isSettingsListFull,
	removeSetting,
	replaceSetting,
	setSettingEnabled
} from './advanced-edits.js';
import { ADVANCED_MAX } from './advanced-validation.js';

function setting(key: string, value = 'v', enabled = true): AdvancedSetting {
	return { key, value, enabled };
}

describe('advanced settings edits', () => {
	it('adds a setting at the end', () => {
		expect(addSetting(setting('c'))([setting('a'), setting('b')]).map(({ key }) => key)).toEqual([
			'a',
			'b',
			'c'
		]);
	});

	it('does not add a key twice nor past the limit', () => {
		const list = [setting('a')];
		expect(addSetting(setting('a', 'other'))(list)).toBe(list);
		const full = Array.from({ length: ADVANCED_MAX }, (_, index) => setting(`k${index}`));
		expect(isSettingsListFull(full)).toBe(true);
		expect(addSetting(setting('new'))(full)).toBe(full);
	});

	it('replaces a setting in its place, switches it and removes it', () => {
		const list = [setting('a'), setting('b'), setting('c')];
		expect(replaceSetting('b', setting('B', 'x'))(list)).toEqual([
			setting('a'),
			setting('B', 'x'),
			setting('c')
		]);
		expect(setSettingEnabled('b', false)(list)[1]).toEqual(setting('b', 'v', false));
		expect(removeSetting('b')(list).map(({ key }) => key)).toEqual(['a', 'c']);
	});

	it('goes to the forms of the aliases and back', () => {
		const original = setting('push.default', 'current', false);
		expect(asAlias(original)).toEqual({ name: 'push.default', command: 'current', enabled: false });
		expect(fromAlias(asAlias(original))).toEqual(original);
	});
});
