import type {
	ButtonAction,
	CatalogModule,
	HapticLevel,
	HapticWaveform,
	MouseControlId,
	WheelMode
} from '#lib/generated/index.js';
import { shortcutLabel } from './shortcut.js';

/** What a person calls each control of a mouse. */
export const CONTROL_LABELS: Readonly<Record<MouseControlId, string>> = {
	left: 'Clic gauche',
	right: 'Clic droit',
	wheelClick: 'Clic de la molette',
	wheelMode: 'Bouton de mode de la molette',
	back: 'Bouton arrière',
	forward: 'Bouton avant',
	gesture: 'Bouton de gestes',
	thumbWheel: 'Molette du pouce',
	hapticPanel: 'Panneau haptique'
};

/** How a control behaves when it has no action of its own (a locked control always does). */
export const NATIVE = 'native';

export type ActionKind = ButtonAction['kind'] | typeof NATIVE;

/** The kinds of action a control can be given, in the order the interface lists them. */
export const ACTION_KINDS: readonly { value: ActionKind; label: string }[] = [
	{ value: NATIVE, label: 'Comportement d’origine' },
	{ value: 'none', label: 'Désactivé' },
	{ value: 'click', label: 'Clic' },
	{ value: 'doubleClick', label: 'Double clic' },
	{ value: 'toggleWheelMode', label: 'Basculer le mode de la molette' },
	{ value: 'radialMenu', label: 'Menu circulaire' },
	{ value: 'module', label: 'Action d’un module' },
	{ value: 'shortcut', label: 'Raccourci clavier' }
];

export const MOUSE_BUTTONS: readonly {
	value: 'left' | 'right' | 'middle' | 'back' | 'forward';
	label: string;
}[] = [
	{ value: 'left', label: 'Gauche' },
	{ value: 'right', label: 'Droit' },
	{ value: 'middle', label: 'Milieu' },
	{ value: 'back', label: 'Arrière' },
	{ value: 'forward', label: 'Avant' }
];

export const WHEEL_MODES: readonly { value: WheelMode; label: string }[] = [
	{ value: 'ratchet', label: 'Crantée' },
	{ value: 'free', label: 'Libre' },
	{ value: 'auto', label: 'Automatique (libre quand on la lance)' }
];

export const HAPTIC_LEVELS: readonly { value: HapticLevel; label: string }[] = [
	{ value: 'off', label: 'Aucune vibration' },
	{ value: 'subtle', label: 'Très légère' },
	{ value: 'low', label: 'Légère' },
	{ value: 'medium', label: 'Moyenne' },
	{ value: 'high', label: 'Forte' }
];

export const HAPTIC_WAVEFORMS: readonly { value: HapticWaveform; label: string }[] = [
	{ value: 'sharpStateChange', label: 'Tic net (changement d’état)' },
	{ value: 'dampStateChange', label: 'Tic doux (changement d’état)' },
	{ value: 'sharpCollision', label: 'Choc net' },
	{ value: 'dampCollision', label: 'Choc sourd' },
	{ value: 'subtleCollision', label: 'Choc léger' },
	{ value: 'whisperCollision', label: 'Choc à peine perceptible' },
	{ value: 'happyAlert', label: 'Alerte joyeuse' },
	{ value: 'angryAlert', label: 'Alerte sèche' },
	{ value: 'completed', label: 'Terminé (montée)' },
	{ value: 'square', label: 'Impulsion carrée' },
	{ value: 'wave', label: 'Vague douce' },
	{ value: 'firework', label: 'Feu d’artifice' },
	{ value: 'mad', label: 'Insistant' },
	{ value: 'knock', label: 'Deux petits coups' },
	{ value: 'jingle', label: 'Clochette' },
	{ value: 'ringing', label: 'Sonnerie' }
];

/** What a person calls a setting the mouse refused (the names the manager reports). */
const SETTING_LABELS: Readonly<Record<string, string>> = {
	sensitivity: 'la sensibilité du pointeur',
	wheel: 'le mode de la molette',
	'high resolution wheel': 'le défilement haute résolution et naturel',
	'thumb wheel': 'le sens de la molette du pouce',
	'haptic strength': 'la force des vibrations',
	button: 'l’action d’un bouton',
	'new configuration': 'la nouvelle configuration',
	'buttons left by a previous run': 'des boutons restés détournés par une exécution précédente'
};

/** The refused setting, in words; an unknown name is shown as it is. */
export function settingLabel(setting: string): string {
	return SETTING_LABELS[setting] ?? setting;
}

/** What a control does by itself, on the mice that have it, as the manager leaves it when it has no action here. */
export const ORIGINAL_LABELS: Readonly<Record<MouseControlId, string>> = {
	left: 'Clic gauche',
	right: 'Clic droit',
	wheelClick: 'Clic milieu',
	wheelMode: 'Cran / libre',
	back: 'Précédent',
	forward: 'Suivant',
	gesture: 'Aucune action',
	thumbWheel: 'Défilement horizontal',
	hapticPanel: 'Aucune action'
};

/**
 * What a control does, in a few words: its action when it has one, else what the mouse does by itself.
 * A module or an action the catalog does not know is shown by its identifier.
 */
export function describeAction(
	action: ButtonAction | undefined,
	original: string,
	catalog: readonly CatalogModule[]
): string {
	switch (action?.kind) {
		case undefined:
			return original;
		case 'none':
			return 'Désactivé';
		case 'click':
			return `Clic ${MOUSE_BUTTONS.find((button) => button.value === action.button)?.label.toLowerCase() ?? action.button}`;
		case 'doubleClick':
			return 'Double clic';
		case 'toggleWheelMode':
			return 'Cran / libre';
		case 'radialMenu':
			return 'Menu circulaire';
		case 'module': {
			const module = catalog.find((entry) => entry.kind === action.module);
			const named = module?.radialActions.find((entry) => entry.id === action.action);
			return `${module?.name ?? action.module} › ${named?.name ?? action.action}`;
		}
		case 'shortcut':
			return `Raccourci ${shortcutLabel(action.keys)}`;
	}
}

/**
 * The shape of each waveform, as twelve strengths from 0 to 9 (one per 60 ms): what the monitor of the haptic
 * tab draws and plays. Indicative: the real patterns are the mouse's own.
 */
export const HAPTIC_PATTERNS: Readonly<Record<HapticWaveform, string>> = {
	sharpStateChange: '900000000000',
	dampStateChange: '642100000000',
	sharpCollision: '962100000000',
	dampCollision: '531000000000',
	subtleCollision: '310000000000',
	whisperCollision: '210000000000',
	happyAlert: '707090000000',
	angryAlert: '909090909000',
	completed: '406080900000',
	square: '999999000000',
	wave: '135797531000',
	firework: '900500300100',
	mad: '919191919100',
	knock: '909000909000',
	jingle: '505050505000',
	ringing: '959595959500'
};

/** How much the mouse shakes in the monitor for each strength (the same relative feel as the real one). */
export const HAPTIC_GAINS: Readonly<Record<HapticLevel, number>> = {
	off: 0,
	subtle: 0.15,
	low: 0.35,
	medium: 0.65,
	high: 1
};
