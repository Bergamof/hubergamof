import { describe, expect, it } from 'vitest';
import type { CatalogModule } from '#lib/generated/index.js';
import {
	CONTEXTUAL_OPTION,
	NAVIGATION_OPTION,
	NO_DIAL_FUNCTION,
	assignmentOption,
	choiceOf,
	existingOption,
	newOption
} from './dial-options.js';

const volume = { id: 'volume', name: 'Contrôle du volume', description: '' };

const audio: CatalogModule = {
	kind: 'audio',
	name: 'Audio',
	description: '',
	capabilities: { key: true },
	dialFunctions: [volume],
	settings: [],
	icon: null,
	settingIcons: {},
	defaultSettings: {}
};

describe('dial options', () => {
	it('round-trip the functions of the manager, existing instances, new modules and none', () => {
		expect(choiceOf(NAVIGATION_OPTION, [audio])).toEqual({ type: 'navigation' });
		expect(choiceOf(CONTEXTUAL_OPTION, [audio])).toEqual({ type: 'contextual' });
		expect(choiceOf(existingOption('audio-2', 'volume'), [audio])).toEqual({
			type: 'existing',
			instance: 'audio-2',
			function: 'volume'
		});
		expect(choiceOf(newOption(audio, volume), [audio])).toEqual({
			type: 'new',
			module: audio,
			function: 'volume'
		});
		expect(choiceOf(NO_DIAL_FUNCTION, [audio])).toEqual({ type: 'none' });
	});

	it('stand for what a dial already does', () => {
		expect(assignmentOption({ kind: 'none' })).toBe(NO_DIAL_FUNCTION);
		expect(assignmentOption({ kind: 'navigation' })).toBe(NAVIGATION_OPTION);
		expect(assignmentOption({ kind: 'contextual' })).toBe(CONTEXTUAL_OPTION);
		const assignment = { kind: 'module', instance: 'audio', function: 'volume' } as const;
		expect(assignmentOption(assignment)).toBe(existingOption('audio', 'volume'));
		expect(choiceOf(assignmentOption(assignment), [audio])).toMatchObject({ type: 'existing' });
	});

	it('read an unknown value as none', () => {
		expect(choiceOf('new:gone/volume', [audio])).toEqual({ type: 'none' });
		expect(choiceOf('new:audio/gone', [audio])).toEqual({ type: 'none' });
		expect(choiceOf('new:audio', [audio])).toEqual({ type: 'none' });
		expect(choiceOf('function:audio', [audio])).toEqual({ type: 'none' });
		expect(choiceOf('garbage', [audio])).toEqual({ type: 'none' });
	});
});
