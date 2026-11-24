import type { AmbientMode, EqualizerPreset, HeadsetLink } from '#lib/generated/index.js';

/** The modes of the ambient sound, in the order they are offered. */
export const AMBIENT_MODES: readonly { value: AmbientMode; label: string }[] = [
	{ value: 'noiseCancelling', label: 'Réduction de bruit' },
	{ value: 'ambient', label: 'Son ambiant' },
	{ value: 'windReduction', label: 'Réduction du vent' },
	{ value: 'off', label: 'Désactivé' }
];

/** Highest ambient level the headphones take. */
export const MAX_AMBIENT_LEVEL = 20;

/** Range of the bass boost and of each band of the equalizer. */
export const EQUALIZER_MIN = -10;
export const EQUALIZER_MAX = 10;

/** The bands of the equalizer from low to high. */
export const EQUALIZER_BANDS: readonly string[] = [
	'400 Hz',
	'1 kHz',
	'2,5 kHz',
	'6,3 kHz',
	'16 kHz'
];

const PRESET_LABELS: Readonly<Record<EqualizerPreset, string>> = {
	off: 'Désactivé',
	bright: 'Brillant',
	excited: 'Dynamique',
	mellow: 'Doux',
	relaxed: 'Détendu',
	vocal: 'Voix',
	trebleBoost: 'Aigus renforcés',
	bassBoost: 'Basses renforcées',
	speech: 'Parole',
	manual: 'Manuel',
	custom1: 'Personnalisé 1',
	custom2: 'Personnalisé 2',
	other: 'Autre'
};

/** Presets a person can ask for: the manual one is reached by moving a band, `other` is not one. */
const CHOOSABLE: readonly EqualizerPreset[] = [
	'off',
	'bright',
	'excited',
	'mellow',
	'relaxed',
	'vocal',
	'trebleBoost',
	'bassBoost',
	'speech',
	'custom1',
	'custom2'
];

/** The presets to list: the ones to choose from, plus the one in use when it is not among them. */
export function presetOptions(
	current: EqualizerPreset
): { value: EqualizerPreset; label: string }[] {
	const listed = CHOOSABLE.includes(current) ? CHOOSABLE : [current, ...CHOOSABLE];
	return listed.map((value) => ({ value, label: PRESET_LABELS[value] }));
}

/** What the link to the headphones is, for a person. */
export const LINK_LABELS: Readonly<Record<HeadsetLink, string>> = {
	idle: 'Pas encore connecté',
	connecting: 'Connexion en cours…',
	connected: 'Connecté',
	released: 'Libéré : une autre application peut utiliser le casque',
	cut: 'Connexion coupée : une autre application (celle de Sony, par exemple) a pris le casque'
};
