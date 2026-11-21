/**
 * Names the manager gives to the keys of a shortcut, by `KeyboardEvent.code` (the position
 * of the key on a US keyboard, whatever the layout, which is what the manager presses).
 */
const MODIFIERS: Readonly<Record<string, string>> = {
	ControlLeft: 'ctrl',
	ControlRight: 'ctrl',
	ShiftLeft: 'shift',
	ShiftRight: 'shift',
	AltLeft: 'alt',
	AltRight: 'alt',
	MetaLeft: 'super',
	MetaRight: 'super'
};

/** Modifiers in the order a shortcut is written. */
const MODIFIER_ORDER = ['ctrl', 'alt', 'shift', 'super'];

const NAMED: Readonly<Record<string, string>> = {
	Enter: 'enter',
	NumpadEnter: 'enter',
	Escape: 'esc',
	Tab: 'tab',
	Space: 'space',
	Backspace: 'backspace',
	Delete: 'delete',
	Insert: 'insert',
	Home: 'home',
	End: 'end',
	PageUp: 'pageup',
	PageDown: 'pagedown',
	ArrowUp: 'up',
	ArrowDown: 'down',
	ArrowLeft: 'left',
	ArrowRight: 'right',
	Minus: 'minus',
	Equal: 'equal',
	Comma: 'comma',
	Period: 'dot',
	Slash: 'slash',
	MediaPlayPause: 'playpause',
	MediaTrackNext: 'nextsong',
	MediaTrackPrevious: 'previoussong',
	AudioVolumeMute: 'mute',
	AudioVolumeUp: 'volumeup',
	AudioVolumeDown: 'volumedown'
};

/** Most keys in a shortcut (the manager refuses more). */
export const MAX_SHORTCUT_KEYS = 6;

/** Name of a modifier key, `undefined` for any other. */
export function modifierName(code: string): string | undefined {
	return MODIFIERS[code];
}

/** Name the manager knows for the key `code`, `undefined` when it cannot press it. */
export function keyName(code: string): string | undefined {
	const letter = /^Key([A-Z])$/.exec(code);
	if (letter) return letter[1].toLowerCase();
	const digit = /^Digit([0-9])$/.exec(code);
	if (digit) return digit[1];
	const fn = /^F([1-9]|1[0-2])$/.exec(code);
	if (fn) return `f${fn[1]}`;
	return MODIFIERS[code] ?? NAMED[code];
}

/** The shortcut written as the manager reads it (`ctrl+shift+m`), modifiers first. */
export function shortcutOf(codes: readonly string[]): string | undefined {
	const names = codes.map(keyName);
	if (names.length === 0 || names.length > MAX_SHORTCUT_KEYS || names.includes(undefined)) {
		return undefined;
	}
	const unique = [...new Set(names as string[])];
	const modifiers = MODIFIER_ORDER.filter((name) => unique.includes(name));
	return [...modifiers, ...unique.filter((name) => !MODIFIER_ORDER.includes(name))].join('+');
}

/**
 * Whether the manager refuses the shortcut: Ctrl+Alt with Delete, Backspace or a function
 * key logs out, ends the session or switches the console.
 */
export function isReserved(shortcut: string): boolean {
	const names = shortcut.split('+');
	return (
		names.includes('ctrl') &&
		names.includes('alt') &&
		names.some(
			(name) => name === 'delete' || name === 'backspace' || /^f([1-9]|1[0-2])$/.test(name)
		)
	);
}

const LABELS: Readonly<Record<string, string>> = {
	ctrl: 'Ctrl',
	shift: 'Maj',
	alt: 'Alt',
	super: 'Super',
	enter: 'Entrée',
	esc: 'Échap',
	tab: 'Tab',
	space: 'Espace',
	backspace: 'Retour arrière',
	delete: 'Suppr',
	insert: 'Inser',
	home: 'Début',
	end: 'Fin',
	pageup: 'Page haut',
	pagedown: 'Page bas',
	up: '↑',
	down: '↓',
	left: '←',
	right: '→',
	minus: '-',
	equal: '=',
	comma: ',',
	dot: '.',
	slash: '/',
	playpause: 'Lecture/Pause',
	nextsong: 'Piste suivante',
	previoussong: 'Piste précédente',
	mute: 'Muet',
	volumeup: 'Volume +',
	volumedown: 'Volume −'
};

/** A shortcut as shown to the person: `ctrl+shift+m` → `Ctrl + Maj + M`. */
export function shortcutLabel(shortcut: string): string {
	return shortcut
		.split('+')
		.map((name) => LABELS[name] ?? name.toUpperCase())
		.join(' + ');
}

/** Other spellings the manager accepts, by the name they stand for. */
const ALIASES: Readonly<Record<string, string>> = {
	maj: 'shift',
	control: 'ctrl',
	meta: 'super',
	win: 'super',
	return: 'enter',
	escape: 'esc',
	del: 'delete'
};

const KNOWN_NAMES: ReadonlySet<string> = new Set([
	...'abcdefghijklmnopqrstuvwxyz0123456789',
	...Array.from({ length: 12 }, (_, index) => `f${index + 1}`),
	...Object.values(MODIFIERS),
	...Object.values(NAMED)
]);

/**
 * The shortcut typed by hand (`Ctrl + Shift + M`, `ctrl+shift+m`) written as the manager
 * reads it, `undefined` when a key is unknown or there are none or too many.
 */
export function normalizeShortcut(text: string): string | undefined {
	const names = text
		.split('+')
		.map((name) => name.trim().toLowerCase())
		.map((name) => ALIASES[name] ?? name);
	if (names.length > MAX_SHORTCUT_KEYS || !names.every((name) => KNOWN_NAMES.has(name))) {
		return undefined;
	}
	const unique = [...new Set(names)];
	const modifiers = MODIFIER_ORDER.filter((name) => unique.includes(name));
	return [...modifiers, ...unique.filter((name) => !MODIFIER_ORDER.includes(name))].join('+');
}

/** The codes of the keys that are not letters or digits and carry a character. */
const CHARACTER_CODES: Readonly<Record<string, string>> = {
	minus: 'Minus',
	equal: 'Equal',
	comma: 'Comma',
	dot: 'Period',
	slash: 'Slash'
};

/** `KeyboardEvent.code` of the key a name stands for, `undefined` when it carries no character. */
function characterCode(name: string): string | undefined {
	if (/^[a-z]$/.test(name)) return `Key${name.toUpperCase()}`;
	if (/^[0-9]$/.test(name)) return `Digit${name}`;
	return CHARACTER_CODES[name];
}

/**
 * The shortcut as the person's keyboard writes it, when it is not the same as the American
 * positions the manager presses (on an AZERTY keyboard, the key `a` carries `Q`): `undefined`
 * when nothing differs or the layout is unknown.
 */
export function shortcutOnLayout(
	shortcut: string,
	layout: ReadonlyMap<string, string> | undefined
): string | undefined {
	if (!layout) return undefined;
	let differs = false;
	const parts = shortcut.split('+').map((name) => {
		const code = characterCode(name);
		const produced = code === undefined ? undefined : layout.get(code)?.toUpperCase();
		const position = shortcutLabel(name);
		if (produced === undefined || produced === position) return position;
		differs = true;
		return produced;
	});
	return differs ? parts.join(' + ') : undefined;
}
