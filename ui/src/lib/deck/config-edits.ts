import type {
	CatalogModule,
	DeviceConfig,
	DeviceTheme,
	DialAssignment,
	DialFunctionId,
	JsonValue,
	ModuleInstance,
	ModuleInstanceId,
	StripConfig
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

/** Highest brightness, in percent: `MAX_BRIGHTNESS` of `hubergamof_protocol` (constants are not exported to TS). */
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

/**
 * The instance every placement of `module` shares: a module that does not allow several
 * instances has at most one on the device, whatever its key and dials. `undefined` when the
 * module may be configured several times or has no instance yet.
 */
export function sharedInstance(
	config: DeviceConfig,
	module: CatalogModule
): ModuleInstance | undefined {
	if (module.capabilities.multipleInstances) return undefined;
	return config.modules.find((instance) => instance.kind === module.kind);
}

/** Whether a key can be added for `module`: it may be on a key, and not already on one if single. */
export function canAddKey(config: DeviceConfig, module: CatalogModule): boolean {
	if (!module.capabilities.key) return false;
	const shared = sharedInstance(config, module);
	return !shared || !config.home.includes(shared.id);
}

/** Whether a dial can get a new instance of `module` (a single-instance one is chosen as existing). */
export function canCreateInstance(config: DeviceConfig, module: CatalogModule): boolean {
	return sharedInstance(config, module) === undefined;
}

/** Whether a dial other than `dial` has a function of a module of `kind`: a module has one dial at most. */
export function kindOnOtherDial(config: DeviceConfig, kind: string, dial: number): boolean {
	return config.dials.some(
		(assignment, index) =>
			index !== dial &&
			assignment.kind === 'module' &&
			config.modules.find((module) => module.id === assignment.instance)?.kind === kind
	);
}

/** The instance a dial function belongs to, if the dial has a function of a module. */
export function dialInstance(assignment: DialAssignment): ModuleInstanceId | undefined {
	return assignment.kind === 'module' ? assignment.instance : undefined;
}

function usedInstanceIds(config: DeviceConfig): Set<ModuleInstanceId> {
	const onDials = config.dials
		.map(dialInstance)
		.filter((id): id is ModuleInstanceId => id !== undefined);
	return new Set<ModuleInstanceId>([...config.home, ...onDials]);
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

/** Sets the theme of the device: keys and touch strip. */
export function setTheme(theme: DeviceTheme): ConfigEdit {
	return (config) => ({ ...config, theme });
}

/**
 * Adds a key at the end of the home page: with a new instance of `module`, or with the instance
 * it already has when it allows a single one (its settings are then shared with its dials).
 * Nothing changes when the module cannot be on a key or already is.
 */
export function addKey(module: CatalogModule): ConfigEdit {
	return (config) => {
		if (!canAddKey(config, module)) return config;
		const shared = sharedInstance(config, module);
		if (shared) return { ...config, home: [...config.home, shared.id] };
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

/**
 * What a dial gets: nothing, a function of the manager, a function of an instance already
 * configured, or a function of a new instance of a module.
 */
export type DialChoice =
	| { type: 'none' }
	| { type: 'navigation' }
	| { type: 'contextual' }
	| { type: 'existing'; instance: ModuleInstanceId; function: DialFunctionId }
	| { type: 'new'; module: CatalogModule; function: DialFunctionId };

/** Assigns dial `dial` according to `choice`; an out-of-range dial changes nothing. */
export function assignDial(dial: number, choice: DialChoice): ConfigEdit {
	return (config) => {
		if (dial < 0 || dial >= config.dials.length) return config;
		const kind =
			choice.type === 'new'
				? choice.module.kind
				: choice.type === 'existing'
					? config.modules.find((module) => module.id === choice.instance)?.kind
					: undefined;
		if (kind !== undefined && kindOnOtherDial(config, kind, dial)) return config;
		const shared = choice.type === 'new' ? sharedInstance(config, choice.module) : undefined;
		const added =
			choice.type === 'new' && !shared ? createInstance(config, choice.module) : undefined;
		const assignment = assignmentOf(choice, added ?? shared);
		return withoutUnusedInstances({
			...config,
			modules: added ? [...config.modules, added] : config.modules,
			dials: config.dials.map((current, index) => (index === dial ? assignment : current))
		});
	};
}

/** `instance`: the one a `new` choice ends up with (created, or the module's single one). */
function assignmentOf(choice: DialChoice, instance: ModuleInstance | undefined): DialAssignment {
	switch (choice.type) {
		case 'navigation':
			return { kind: 'navigation' };
		case 'contextual':
			return { kind: 'contextual' };
		case 'existing':
			return { kind: 'module', instance: choice.instance, function: choice.function };
		case 'new':
			return {
				kind: 'module',
				instance: instance?.id ?? choice.module.kind,
				function: choice.function
			};
		default:
			return { kind: 'none' };
	}
}

/** Changes some options of the touch strip. */
export function setStrip(changes: Partial<StripConfig>): ConfigEdit {
	return (config) => ({ ...config, strip: { ...config.strip, ...changes } });
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
