/** Remembers the device the user last chose, for the next visit. */
export interface DevicePreference {
	load(): string | null;
	save(serial: string): void;
}

const STORAGE_KEY = 'hubergamof.selectedDevice';

/**
 * Backed by `localStorage`. Storage may be unavailable (private browsing, blocked site
 * data): the preference is then simply not remembered.
 */
export const browserDevicePreference: DevicePreference = {
	load() {
		try {
			return localStorage.getItem(STORAGE_KEY);
		} catch {
			return null;
		}
	},
	save(serial) {
		try {
			localStorage.setItem(STORAGE_KEY, serial);
		} catch {
			// Not remembered: the first device is selected next time.
		}
	}
};
