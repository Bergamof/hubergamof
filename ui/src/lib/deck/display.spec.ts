import { describe, expect, it } from 'vitest';
import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
import { instanceName, keyImage, moduleName } from './display.js';

const audio: CatalogModule = {
	kind: 'audio',
	name: 'Audio',
	description: '',
	capabilities: { key: true, dial: true, touchZone: true },
	settings: [],
	icon: null,
	defaultSettings: {}
};

function instance(label: string | null, kind = 'audio'): ModuleInstance {
	return { id: kind, kind, label, image: null, settings: {} };
}

describe('instanceName', () => {
	it('prefers the label, then the module name', () => {
		expect(instanceName(instance('Micro'), audio)).toBe('Micro');
		expect(instanceName(instance(null), audio)).toBe('Audio');
		expect(instanceName(instance(null, 'intellij'), undefined)).toBe('IntelliJ');
	});
});

describe('moduleName', () => {
	it('names upcoming modules, and falls back to the kind', () => {
		expect(moduleName('claude', undefined)).toBe('Claude');
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
