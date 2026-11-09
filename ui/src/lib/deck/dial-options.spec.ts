import { describe, expect, it } from 'vitest';
import type { CatalogModule } from '#lib/generated/index.js';
import { NO_DIAL_MODULE, choiceOf, existingOption, newOption } from './dial-options.js';

const clock: CatalogModule = {
	kind: 'clock',
	name: 'Horloge',
	description: '',
	capabilities: { key: false, dial: true, touchZone: true },
	settings: [],
	icon: null,
	settingIcons: {},
	defaultSettings: {}
};

describe('dial options', () => {
	it('round-trip existing instances, new modules and none', () => {
		expect(choiceOf(existingOption('audio-2'), [clock])).toEqual({
			type: 'existing',
			id: 'audio-2'
		});
		expect(choiceOf(newOption(clock), [clock])).toEqual({ type: 'new', module: clock });
		expect(choiceOf(NO_DIAL_MODULE, [clock])).toEqual({ type: 'none' });
	});

	it('read an unknown value as none', () => {
		expect(choiceOf('new:gone', [clock])).toEqual({ type: 'none' });
		expect(choiceOf('garbage', [clock])).toEqual({ type: 'none' });
	});
});
