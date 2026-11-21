import type {
	ButtonAction,
	HapticEventSetting,
	HapticLevel,
	MouseConfig,
	MouseControlId,
	RadialEntryConfig,
	RadialMenuConfig,
	RadialPrimary,
	ThumbWheelSettings,
	WheelSettings
} from '#lib/generated/index.js';

/** Sensitivity range and step, as the manager accepts them (`DPI_RANGE`, `DPI_STEP`). */
export const DPI_MIN = 200;
export const DPI_MAX = 8000;
export const DPI_STEP = 50;
/** Speed from which the wheel leaves its ratchet in automatic mode. */
export const WHEEL_THRESHOLD_MIN = 1;
export const WHEEL_THRESHOLD_MAX = 50;
/** Speeds of the thumb wheel. */
export const THUMB_SPEED_MIN = 1;
export const THUMB_SPEED_MAX = 10;
/** Hold time that opens the radial menu for good, in milliseconds (`HOLD_RANGE`). */
export const HOLD_MIN_MS = 150;
export const HOLD_MAX_MS = 500;
/** Entries shown by the radial menu: fewer is not a menu, more cannot be hit. */
export const MIN_SHOWN_ENTRIES = 3;
export const MAX_SHOWN_ENTRIES = 8;

/** A change of the configuration of a mouse. */
export type MouseEdit = (config: MouseConfig) => MouseConfig;
/** A change of the radial menu. */
export type RadialEdit = (config: RadialMenuConfig) => RadialMenuConfig;

/**
 * Gives `control` an action; `null` gives it back to the mouse (no entry). The menu opens
 * from one control only, so giving it to this one takes it from the others.
 */
export function bindControl(control: MouseControlId, action: ButtonAction | null): MouseEdit {
	return (config) => {
		const bindings = { ...config.bindings };
		if (action?.kind === 'radialMenu') {
			for (const [id, existing] of Object.entries(bindings)) {
				if (existing?.kind === 'radialMenu') delete bindings[id as MouseControlId];
			}
		}
		if (action === null) delete bindings[control];
		else bindings[control] = action;
		return { ...config, bindings };
	};
}

export function setDpi(dpi: number): MouseEdit {
	return (config) => ({ ...config, pointer: { ...config.pointer, dpi } });
}

export function setWheel(change: Partial<WheelSettings>): MouseEdit {
	return (config) => ({ ...config, wheel: { ...config.wheel, ...change } });
}

export function setThumbWheel(change: Partial<ThumbWheelSettings>): MouseEdit {
	return (config) => ({ ...config, thumbWheel: { ...config.thumbWheel, ...change } });
}

export function setHapticLevel(level: HapticLevel): MouseEdit {
	return (config) => ({ ...config, haptics: { ...config.haptics, level } });
}

/** Key of the haptic event `event` of `module` in the configuration of a mouse. */
export function hapticKey(module: string, event: string): string {
	return `${module}.${event}`;
}

/** Sets how a haptic event is played; `null` goes back to the module's default. */
export function setHapticEvent(key: string, setting: HapticEventSetting | null): MouseEdit {
	return (config) => {
		const events = { ...config.haptics.events };
		if (setting === null) delete events[key];
		else events[key] = setting;
		return { ...config, haptics: { ...config.haptics, events } };
	};
}

export function setHoldMs(holdMs: number): RadialEdit {
	return (config) => ({ ...config, holdMs });
}

export function setRadialOptions(
	change: Partial<Pick<RadialMenuConfig, 'animate' | 'haptics'>>
): RadialEdit {
	return (config) => ({ ...config, ...change });
}

function updateEntry(module: string, change: Partial<RadialEntryConfig>): RadialEdit {
	return (config) => ({
		...config,
		entries: config.entries.map((entry) =>
			entry.module === module ? { ...entry, ...change } : entry
		)
	});
}

export function setEntryEnabled(module: string, enabled: boolean): RadialEdit {
	return updateEntry(module, { enabled });
}

export function setEntryPrimary(module: string, primary: RadialPrimary): RadialEdit {
	return updateEntry(module, { primary });
}

/** Moves the entry of `module` by `offset` places (negative: earlier), within the list. */
export function moveEntry(module: string, offset: number): RadialEdit {
	return (config) => {
		const from = config.entries.findIndex((entry) => entry.module === module);
		const to = Math.min(Math.max(from + offset, 0), config.entries.length - 1);
		if (from < 0 || from === to) return config;
		const entries = [...config.entries];
		const [moved] = entries.splice(from, 1);
		entries.splice(to, 0, moved);
		return { ...config, entries };
	};
}

/** Adds `module` to the menu, shown, its click doing `action`; no-op when already listed. */
export function addEntry(module: string, action: string): RadialEdit {
	return (config) =>
		config.entries.some((entry) => entry.module === module)
			? config
			: {
					...config,
					entries: [
						...config.entries,
						{ module, enabled: true, primary: { kind: 'action', action } }
					]
				};
}

export function removeEntry(module: string): RadialEdit {
	return (config) => ({
		...config,
		entries: config.entries.filter((entry) => entry.module !== module)
	});
}

/** How many entries the menu shows. */
export function shownCount(config: RadialMenuConfig): number {
	return config.entries.filter((entry) => entry.enabled).length;
}

/** Whether the number of entries shown makes a menu the manager accepts. */
export function isShownCountValid(config: RadialMenuConfig): boolean {
	const shown = shownCount(config);
	return shown >= MIN_SHOWN_ENTRIES && shown <= MAX_SHOWN_ENTRIES;
}
