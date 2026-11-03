import { afterEach, describe, expect, it, vi } from 'vitest';
import { browserDevicePreference } from './device-preference.js';

function memoryStorage(): Pick<Storage, 'getItem' | 'setItem'> {
	const items = new Map<string, string>();
	return {
		getItem: (key) => items.get(key) ?? null,
		setItem: (key, value) => void items.set(key, value)
	};
}

const unavailable: Pick<Storage, 'getItem' | 'setItem'> = {
	getItem: () => {
		throw new Error('blocked');
	},
	setItem: () => {
		throw new Error('blocked');
	}
};

describe('browserDevicePreference', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('remembers the device in the local storage', () => {
		vi.stubGlobal('localStorage', memoryStorage());
		expect(browserDevicePreference.load()).toBeNull();
		browserDevicePreference.save('ABC');
		expect(browserDevicePreference.load()).toBe('ABC');
	});

	it('forgets quietly when the storage is unavailable', () => {
		vi.stubGlobal('localStorage', unavailable);
		expect(() => browserDevicePreference.save('ABC')).not.toThrow();
		expect(browserDevicePreference.load()).toBeNull();
	});
});
