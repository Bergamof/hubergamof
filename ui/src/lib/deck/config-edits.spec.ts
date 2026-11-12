import { describe, expect, it } from 'vitest';
import type { CatalogModule, DeviceConfig, ModuleInstance } from '#lib/generated/index.js';
import {
	addKey,
	assignDial,
	createInstance,
	moveKey,
	newInstanceId,
	removeKey,
	removedInstance,
	setBrightness,
	setKeyImage,
	setStrip,
	setTheme,
	updateInstance
} from './config-edits.js';

function instance(id: string, kind = id): ModuleInstance {
	return { id, kind, label: null, image: null, settings: {} };
}

function config(overrides: Partial<DeviceConfig> = {}): DeviceConfig {
	return {
		schemaVersion: 3,
		serial: 'DEMO',
		model: 'plus',
		brightness: 50,
		home: ['audio', 'pomodoro', 'docker'],
		modules: [instance('audio'), instance('pomodoro'), instance('docker'), instance('extra')],
		dials: [
			{ kind: 'module', instance: 'extra', function: 'turn' },
			{ kind: 'module', instance: 'pomodoro', function: 'control' },
			{ kind: 'navigation' },
			{ kind: 'none' }
		],
		strip: {
			showSeconds: true,
			showPomodoroProgress: true,
			systemRefreshSeconds: 2,
			showHistory: true,
			loadAlertPercent: 85
		},
		theme: 'light',
		revision: 7,
		...overrides
	};
}

const pomodoro: CatalogModule = {
	kind: 'pomodoro',
	name: 'Pomodoro',
	description: '',
	capabilities: { key: true },
	dialFunctions: [],
	icon: null,
	settingIcons: {},
	defaultSettings: { workMinutes: 25, autoStart: false, computed: 'by the module' },
	settings: [
		{
			key: 'workMinutes',
			label: 'Travail',
			help: null,
			kind: { type: 'integer', default: 25, min: 1, max: 120, unit: 'min' }
		},
		{ key: 'autoStart', label: 'Enchaîner', help: null, kind: { type: 'boolean', default: false } }
	]
};

describe('newInstanceId', () => {
	it('uses the kind while free, then numbers the next instances', () => {
		expect(newInstanceId(config(), 'claude')).toBe('claude');
		expect(newInstanceId(config(), 'audio')).toBe('audio-2');
		const withSecond = config({ modules: [instance('audio'), instance('audio-2', 'audio')] });
		expect(newInstanceId(withSecond, 'audio')).toBe('audio-3');
	});
});

describe('createInstance', () => {
	it('takes the settings the module chooses for a new instance, and no override', () => {
		const created = createInstance(config(), pomodoro);
		expect(created).toEqual({
			id: 'pomodoro-2',
			kind: 'pomodoro',
			label: null,
			image: null,
			settings: { workMinutes: 25, autoStart: false, computed: 'by the module' }
		});
		expect(created.settings).not.toBe(pomodoro.defaultSettings);
	});
});

describe('edits', () => {
	it('keep the revision for the manager to check, and never mutate their input', () => {
		const original = config();
		const frozen = structuredClone(original);
		const edited = removeKey(0)(original);
		expect(edited.revision).toBe(7);
		expect(original).toEqual(frozen);
	});

	it('round and clamp the brightness', () => {
		expect(setBrightness(42.6)(config()).brightness).toBe(43);
		expect(setBrightness(140)(config()).brightness).toBe(100);
		expect(setBrightness(-3)(config()).brightness).toBe(0);
	});

	it('add a key at the end with its new instance', () => {
		const edited = addKey(pomodoro)(config());
		expect(edited.home).toEqual(['audio', 'pomodoro', 'docker', 'pomodoro-2']);
		expect(edited.modules.at(-1)).toEqual(createInstance(config(), pomodoro));
	});

	it('remove a key and its instance unless a dial still uses it', () => {
		const withoutAudio = removeKey(0)(config());
		expect(withoutAudio.home).toEqual(['pomodoro', 'docker']);
		expect(withoutAudio.modules.map((module) => module.id)).toEqual([
			'pomodoro',
			'docker',
			'extra'
		]);

		const withoutPomodoroKey = removeKey(1)(config());
		expect(withoutPomodoroKey.home).toEqual(['audio', 'docker']);
		expect(withoutPomodoroKey.modules.map((module) => module.id)).toContain('pomodoro');
	});

	it('move a key and ignore out-of-range moves', () => {
		expect(moveKey(0, 2)(config()).home).toEqual(['pomodoro', 'docker', 'audio']);
		expect(moveKey(2, 0)(config()).home).toEqual(['docker', 'audio', 'pomodoro']);
		const original = config();
		expect(moveKey(0, 3)(original)).toBe(original);
		expect(moveKey(-1, 0)(original)).toBe(original);
	});

	it('assign a dial a function of the manager, of an existing instance or of a new one', () => {
		const navigation = assignDial(3, { type: 'navigation' })(config());
		expect(navigation.dials[3]).toEqual({ kind: 'navigation' });
		const contextual = assignDial(3, { type: 'contextual' })(config());
		expect(contextual.dials[3]).toEqual({ kind: 'contextual' });

		const existing = assignDial(3, { type: 'existing', instance: 'audio', function: 'volume' })(
			config()
		);
		expect(existing.dials[3]).toEqual({ kind: 'module', instance: 'audio', function: 'volume' });
		expect(existing.modules).toHaveLength(4);

		const added = assignDial(3, { type: 'new', module: pomodoro, function: 'control' })(config());
		expect(added.dials[3]).toEqual({
			kind: 'module',
			instance: 'pomodoro-2',
			function: 'control'
		});
		expect(added.modules.map((module) => module.id)).toContain('pomodoro-2');

		const cleared = assignDial(0, { type: 'none' })(config());
		expect(cleared.dials[0]).toEqual({ kind: 'none' });
		expect(cleared.modules.map((module) => module.id)).not.toContain('extra');

		const original = config();
		expect(assignDial(4, { type: 'navigation' })(original)).toBe(original);
	});

	it('change some options of the strip and keep the others', () => {
		const edited = setStrip({ showSeconds: false })(config());
		expect(edited.strip).toEqual({
			...config().strip,
			showSeconds: false
		});
		expect(edited.revision).toBe(7);
	});

	it('set the theme of the device and keep the strip options', () => {
		const edited = setTheme('dark')(config());
		expect(edited.theme).toBe('dark');
		expect(edited.strip).toEqual(config().strip);
		expect(edited.revision).toBe(7);
	});

	it('replace the label and settings of one instance', () => {
		const edited = updateInstance('audio', { label: 'Volume', settings: { step: 5 } })(config());
		expect(edited.modules[0]).toEqual({
			...instance('audio'),
			label: 'Volume',
			settings: { step: 5 }
		});
		expect(edited.modules[1]).toEqual(instance('pomodoro'));
	});
});

describe('removedInstance', () => {
	it('names the instance a change would delete, if any', () => {
		expect(removedInstance(config(), assignDial(0, { type: 'none' }))?.id).toBe('extra');
		expect(removedInstance(config(), assignDial(1, { type: 'none' }))).toBeUndefined();
		expect(removedInstance(config(), assignDial(2, { type: 'none' }))).toBeUndefined();
		expect(removedInstance(config(), removeKey(0))?.id).toBe('audio');
		expect(removedInstance(config(), setBrightness(10))).toBeUndefined();
	});
});

describe('setKeyImage', () => {
	it('sets or clears the image of one instance', () => {
		const withImage = setKeyImage('audio', 'abc.png')(config());
		expect(withImage.modules[0].image).toBe('abc.png');
		expect(withImage.modules[1].image).toBeNull();
		expect(setKeyImage('audio', null)(withImage).modules[0].image).toBeNull();
	});
});
