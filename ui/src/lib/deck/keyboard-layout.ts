/** What a key produces on the keyboard of the person, by `KeyboardEvent.code`. */
export type KeyboardLayout = ReadonlyMap<string, string>;

interface KeyboardApi {
	getLayoutMap?: () => Promise<KeyboardLayout>;
}

/**
 * The layout of the keyboard, where the browser tells it (`navigator.keyboard`, Chromium): `undefined`
 * elsewhere, or when the browser refuses. Nothing is sent anywhere: it only reads the local layout.
 */
export async function loadKeyboardLayout(
	keyboard: KeyboardApi | undefined = (navigator as { keyboard?: KeyboardApi }).keyboard
): Promise<KeyboardLayout | undefined> {
	try {
		return await keyboard?.getLayoutMap?.();
	} catch {
		return undefined;
	}
}

const STORAGE_KEY = 'hubergamof.keyboard-layout';
/** The keys that carry a character, by position: the only ones a shortcut can name by it. */
const CHARACTER_KEY = /^(Key[A-Z]|Digit[0-9]|Minus|Equal|Comma|Period|Slash)$/;

/** What a key press tells of the layout: where it was, and what it wrote. */
export interface KeyPress {
	code: string;
	key: string;
	ctrlKey: boolean;
	altKey: boolean;
	metaKey: boolean;
}

/**
 * The layout, learned from the keys pressed (browsers that hide `navigator.keyboard`, like Brave, do not
 * tell it): a key pressed alone writes its character, whatever the layout. `undefined` when the press
 * teaches nothing (a modifier is held, the key is not a character key, or it is already known).
 */
export function learnKey(known: KeyboardLayout, press: KeyPress): KeyboardLayout | undefined {
	if (press.ctrlKey || press.altKey || press.metaKey) return undefined;
	if (!CHARACTER_KEY.test(press.code) || press.key.length !== 1) return undefined;
	const written = press.key.toLowerCase();
	if (known.get(press.code) === written) return undefined;
	return new Map(known).set(press.code, written);
}

/** What the layout of the keys in `first` and `second` is together; `second` wins where both know. */
export function mergeLayouts(
	first: KeyboardLayout | undefined,
	second: KeyboardLayout | undefined
): KeyboardLayout {
	return new Map([...(first ?? []), ...(second ?? [])]);
}

/** The layout learned in earlier visits; empty where nothing was kept or the storage is refused. */
export function readLearnedLayout(
	storage: Pick<Storage, 'getItem'> | undefined = safeStorage()
): KeyboardLayout {
	try {
		const parsed: unknown = JSON.parse(storage?.getItem(STORAGE_KEY) ?? '[]');
		if (!Array.isArray(parsed)) return new Map();
		return new Map(
			parsed.filter(
				(entry): entry is [string, string] =>
					Array.isArray(entry) &&
					typeof entry[0] === 'string' &&
					CHARACTER_KEY.test(entry[0]) &&
					typeof entry[1] === 'string' &&
					entry[1].length === 1
			)
		);
	} catch {
		return new Map();
	}
}

/** Keeps what was learned for the next visits; a refused storage is no harm. */
export function saveLearnedLayout(
	layout: KeyboardLayout,
	storage: Pick<Storage, 'setItem'> | undefined = safeStorage()
): void {
	try {
		storage?.setItem(STORAGE_KEY, JSON.stringify([...layout]));
	} catch {
		// The layout is learned again next time.
	}
}

function safeStorage(): Storage | undefined {
	try {
		return localStorage;
	} catch {
		return undefined;
	}
}

/** A layout guessed from the language, with its name. */
export interface GuessedLayout {
	name: string;
	layout: KeyboardLayout;
}

/** Every letter key writing its own letter: the layouts of a language then differ by a few. */
function ownLetters(): [string, string][] {
	return [...'abcdefghijklmnopqrstuvwxyz'].map((letter) => [`Key${letter.toUpperCase()}`, letter]);
}

const AZERTY: GuessedLayout = {
	name: 'AZERTY',
	layout: new Map([
		...ownLetters(),
		['KeyA', 'q'],
		['KeyQ', 'a'],
		['KeyW', 'z'],
		['KeyZ', 'w'],
		['KeyM', ','],
		['Comma', ';'],
		['Period', ':'],
		['Slash', '!'],
		['Digit1', '&'],
		['Digit2', 'é'],
		['Digit3', '"'],
		['Digit4', "'"],
		['Digit5', '('],
		['Digit6', '-'],
		['Digit7', 'è'],
		['Digit8', '_'],
		['Digit9', 'ç'],
		['Digit0', 'à'],
		['Minus', ')'],
		['Equal', '=']
	])
};

const QWERTZ: GuessedLayout = {
	name: 'QWERTZ',
	layout: new Map([
		...ownLetters(),
		['Digit1', '1'],
		['Digit2', '2'],
		['Digit3', '3'],
		['Digit4', '4'],
		['Digit5', '5'],
		['Digit6', '6'],
		['Digit7', '7'],
		['Digit8', '8'],
		['Digit9', '9'],
		['Digit0', '0'],
		['Comma', ','],
		['Period', '.'],
		['Equal', '´'],
		['KeyY', 'z'],
		['KeyZ', 'y'],
		['Minus', 'ß'],
		['Slash', '-']
	])
};

/** Languages whose usual keyboard is not the American one (a Canadian French keyboard is QWERTY: left out). */
const LAYOUTS_BY_LANGUAGE: Readonly<Record<string, GuessedLayout>> = {
	fr: AZERTY,
	'fr-fr': AZERTY,
	'fr-be': AZERTY,
	de: QWERTZ,
	'de-de': QWERTZ,
	'de-at': QWERTZ,
	'de-ch': QWERTZ,
	'fr-ch': QWERTZ
};

/**
 * The layout the language of the browser usually goes with, when it is not the American one. Only a guess:
 * a French browser can have a QWERTY or Bépo keyboard. The layout the browser tells, or the keys pressed,
 * replace it.
 */
export function layoutForLocale(locale: string | undefined): GuessedLayout | undefined {
	const normalised = locale?.toLowerCase().replace('_', '-');
	return normalised === undefined ? undefined : LAYOUTS_BY_LANGUAGE[normalised];
}
