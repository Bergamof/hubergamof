import type {
	ButtonAction,
	HapticLevel,
	HapticWaveform,
	MouseControlId,
	WheelMode
} from '#lib/generated/index.js';

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
