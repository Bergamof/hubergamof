import type {
	CatalogModule,
	MouseConfig,
	MouseInfo,
	RadialMenuConfig
} from '#lib/generated/index.js';

/** A module of the catalog as the mouse pages need it. */
export function catalogModule(
	kind: string,
	name: string,
	radialActions: [string, string][] = [],
	hapticEvents: [string, string][] = []
): CatalogModule {
	return {
		kind,
		name,
		description: '',
		capabilities: {
			key: true,
			multipleInstances: false,
			usableWhenLocked: false,
			lockSafePages: false
		},
		dialFunctions: [],
		settings: [],
		icon: null,
		settingIcons: {},
		defaultSettings: {},
		radialActions: radialActions.map(([id, label]) => ({
			id,
			name: label,
			description: `${label}.`
		})),
		hapticEvents: hapticEvents.map(([id, label]) => ({
			id,
			name: label,
			description: `Quand : ${label}.`,
			waveform: 'completed'
		}))
	};
}

export const POMODORO = catalogModule(
	'pomodoro',
	'Pomodoro',
	[
		['toggle', 'Démarrer ou mettre en pause'],
		['skip', 'Passer à l’étape suivante']
	],
	[['step-end', 'Fin d’étape']]
);
export const AUDIO = catalogModule('audio', 'Audio', [['mute-microphone', 'Couper le micro']]);
export const CATALOG: CatalogModule[] = [POMODORO, AUDIO, catalogModule('docker', 'Docker')];

export function mouseConfig(overrides: Partial<MouseConfig> = {}): MouseConfig {
	return {
		schemaVersion: 1,
		serial: 'mx-1',
		model: 'mxMaster4',
		bindings: {},
		pointer: { dpi: 1600 },
		wheel: { mode: 'ratchet', autoThreshold: 20, natural: false, hiRes: true },
		thumbWheel: { speed: 5, invert: false },
		haptics: { level: 'medium', events: {} },
		revision: 0,
		...overrides
	};
}

export function mouseInfo(overrides: Partial<MouseInfo> = {}): MouseInfo {
	return {
		serial: 'mx-1',
		model: 'mxMaster4',
		displayName: 'MX Master 4',
		capabilities: {
			controls: [
				{ id: 'left', kind: 'button', locked: true },
				{ id: 'right', kind: 'button', locked: true },
				{ id: 'wheelClick', kind: 'button', locked: false },
				{ id: 'back', kind: 'button', locked: false },
				{ id: 'thumbWheel', kind: 'rotation', locked: false }
			],
			thumbWheel: true,
			haptics: true
		},
		...overrides
	};
}

export function radialMenu(overrides: Partial<RadialMenuConfig> = {}): RadialMenuConfig {
	const entry = (module: string, action: string, enabled = true) => ({
		module,
		enabled,
		primary: { kind: 'action' as const, action }
	});
	return {
		schemaVersion: 1,
		holdMs: 280,
		animate: true,
		haptics: true,
		entries: [
			entry('pomodoro', 'toggle'),
			entry('audio', 'mute-microphone'),
			entry('teams', 'mute'),
			entry('intellij', 'open')
		],
		revision: 0,
		...overrides
	};
}
