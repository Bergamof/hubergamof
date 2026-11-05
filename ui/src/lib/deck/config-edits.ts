import type {
	CatalogModule,
	DeviceConfig,
	JsonValue,
	ModuleInstance,
	ModuleInstanceId
} from '#lib/generated/index.js';
import { cloneJson } from './json.js';

/**
 * Pure edits of a device configuration. Each returns a new configuration (the input is
 * never mutated) with the same `revision`: the manager increments it when it accepts the
 * update. Instances referenced neither by the home page nor by a dial are dropped.
 */
export type ConfigEdit = (config: DeviceConfig) => DeviceConfig;

/** Instance fields the user edits in the settings form. */
export interface InstanceChanges {
	label: string | null;
	settings: JsonValue;
}

/** Highest brightness, in percent: `MAX_BRIGHTNESS` of `bergadeck_protocol` (constants are not exported to TS). */
export const MAX_BRIGHTNESS = 100;

/**
 * Identifier for a new instance of `kind`: the kind itself when free (like the default
 * layout), otherwise `<kind>-2`, `<kind>-3`…
 */
export function newInstanceId(config: DeviceConfig, kind: string): ModuleInstanceId {
	const taken = new Set(config.modules.map((module) => module.id));
	if (!taken.has(kind)) return kind;
	let suffix = 2;
	while (taken.has(`${kind}-${suffix}`)) suffix += 1;
	return `${kind}-${suffix}`;
}

/** A new, unsaved instance of `module` with its default settings. */
export function createInstance(config: DeviceConfig, module: CatalogModule): ModuleInstance {
	return {
		id: newInstanceId(config, module.kind),
		kind: module.kind,
		label: null,
		image: null,
		settings: cloneJson(module.defaultSettings)
	};
}

function usedInstanceIds(config: DeviceConfig): Set<ModuleInstanceId> {
	return new Set<ModuleInstanceId>([
		...config.home,
		...config.dials.filter((id): id is ModuleInstanceId => id !== null)
	]);
}

/** Drops the instances no longer placed on a key or a dial. */
function withoutUnusedInstances(config: DeviceConfig): DeviceConfig {
	const used = usedInstanceIds(config);
	return { ...config, modules: config.modules.filter((module) => used.has(module.id)) };
}

/** The instance (with its settings) `change` would delete, being placed nowhere else. */
export function removedInstance(
	config: DeviceConfig,
	change: ConfigEdit
): ModuleInstance | undefined {
	const kept = usedInstanceIds(change(config));
	return config.modules.find(
		(module) => usedInstanceIds(config).has(module.id) && !kept.has(module.id)
	);
}

/** Sets the brightness, rounded and clamped to `0..=MAX_BRIGHTNESS`. */
export function setBrightness(brightness: number): ConfigEdit {
	const clamped = Math.min(MAX_BRIGHTNESS, Math.max(0, Math.round(brightness)));
	return (config) => ({ ...config, brightness: clamped });
}

/** Adds a key with a new instance of `module` at the end of the home page. */
export function addKey(module: CatalogModule): ConfigEdit {
	return (config) => {
		const instance = createInstance(config, module);
		return {
			...config,
			modules: [...config.modules, instance],
			home: [...config.home, instance.id]
		};
	};
}

/** Removes the key at `position` of the home page. */
export function removeKey(position: number): ConfigEdit {
	return (config) =>
		withoutUnusedInstances({
			...config,
			home: config.home.filter((_, index) => index !== position)
		});
}

/** Moves the key at `from` to `to` on the home page; out-of-range positions change nothing. */
export function moveKey(from: number, to: number): ConfigEdit {
	return (config) => {
		const inRange = (position: number) => position >= 0 && position < config.home.length;
		if (from === to || !inRange(from) || !inRange(to)) return config;
		const home = [...config.home];
		const [moved] = home.splice(from, 1);
		home.splice(to, 0, moved);
		return { ...config, home };
	};
}

/** What a dial gets: nothing, an instance already configured, or a new instance of a module. */
export type DialChoice =
	| { type: 'none' }
	| { type: 'existing'; id: ModuleInstanceId }
	| { type: 'new'; module: CatalogModule };

/** Assigns dial `dial` according to `choice`; an out-of-range dial changes nothing. */
export function assignDial(dial: number, choice: DialChoice): ConfigEdit {
	return (config) => {
		if (dial < 0 || dial >= config.dials.length) return config;
		const added = choice.type === 'new' ? createInstance(config, choice.module) : undefined;
		const id = added?.id ?? (choice.type === 'existing' ? choice.id : null);
		return withoutUnusedInstances({
			...config,
			modules: added ? [...config.modules, added] : config.modules,
			dials: config.dials.map((current, index) => (index === dial ? id : current))
		});
	};
}

/** Replaces the label and settings of instance `id`. */
export function updateInstance(id: ModuleInstanceId, changes: InstanceChanges): ConfigEdit {
	return (config) => ({
		...config,
		modules: config.modules.map((module) =>
			module.id === id ? { ...module, label: changes.label, settings: changes.settings } : module
		)
	});
}

/** Shows image `name` (stored by the manager) on the key of instance `id`, or its module icon (`null`). */
export function setKeyImage(id: ModuleInstanceId, name: string | null): ConfigEdit {
	return (config) => ({
		...config,
		modules: config.modules.map((module) =>
			module.id === id ? { ...module, image: name } : module
		)
	});
}
