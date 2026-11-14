import { describe, expect, it } from 'vitest';
import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
import { dialFunctionName, dialSummary, instanceName, keyImage, moduleName } from './display.js';

const audio: CatalogModule = {
	kind: 'audio',
	name: 'Audio',
	description: '',
	capabilities: { key: true, multipleInstances: false, usableWhenLocked: false },
	dialFunctions: [{ id: 'volume', name: 'Contrôle du volume', description: '' }],
	settings: [],
	icon: null,
	settingIcons: {},
	defaultSettings: {}
};

function instance(label: string | null, kind = 'audio'): ModuleInstance {
	return { id: kind, kind, label, image: null, settings: {} };
}

describe('instanceName', () => {
	it('prefers the label, then the module name', () => {
		expect(instanceName(instance('Micro'), audio)).toBe('Micro');
		expect(instanceName(instance(null), audio)).toBe('Audio');
		expect(instanceName(instance(null, 'teleporter'), undefined)).toBe('teleporter');
	});
});

describe('moduleName', () => {
	it('falls back to the kind of a module missing from the catalog', () => {
		expect(moduleName('mystery', undefined)).toBe('mystery');
		expect(moduleName('constructor', undefined)).toBe('constructor');
		expect(moduleName('audio', audio)).toBe('Audio');
	});
});

describe('keyImage', () => {
	it('prefers the image of the instance, then the module icon', () => {
		const iconic = { ...audio, icon: 'data:image/png;base64,AA==' };
		expect(keyImage('DECK', { ...instance(null), image: 'a.png' }, iconic)).toBe(
			'/api/devices/DECK/images/a.png'
		);
		expect(keyImage('DECK', instance(null), iconic)).toBe('data:image/png;base64,AA==');
		expect(keyImage('DECK', instance(null), undefined)).toBeNull();
	});
});

describe('dialSummary', () => {
	const instances = new Map([['audio', instance(null)]]);
	const moduleOf = (kind: string) => (kind === 'audio' ? audio : undefined);

	it('names the functions of the manager and says what their reminder shows', () => {
		expect(dialSummary({ kind: 'none' }, instances, moduleOf)).toEqual({
			name: 'Rien',
			rotate: null,
			press: null,
			assigned: false,
			missing: false
		});
		expect(dialSummary({ kind: 'navigation' }, instances, moduleOf)).toMatchObject({
			name: 'Navigation',
			rotate: 'Navigation',
			press: 'Retour'
		});
		expect(dialSummary({ kind: 'contextual' }, instances, moduleOf)).toMatchObject({
			name: 'Fonction contextuelle',
			rotate: 'Selon la page',
			assigned: true
		});
	});

	it('names a function of a module, flagging a module this manager lacks', () => {
		const volume = { kind: 'module', instance: 'audio', function: 'volume' } as const;
		expect(dialSummary(volume, instances, moduleOf)).toMatchObject({
			name: 'Contrôle du volume',
			missing: false
		});
		const ghost = new Map([['ghost', instance(null, 'ghost')]]);
		expect(
			dialSummary({ kind: 'module', instance: 'ghost', function: 'x' }, ghost, moduleOf)
		).toMatchObject({ name: 'x', missing: true });
	});
});

describe('dialFunctionName', () => {
	it('falls back to the identifier of an unknown function', () => {
		expect(dialFunctionName(audio, 'volume')).toBe('Contrôle du volume');
		expect(dialFunctionName(audio, 'other')).toBe('other');
		expect(dialFunctionName(undefined, 'volume')).toBe('volume');
	});
});
