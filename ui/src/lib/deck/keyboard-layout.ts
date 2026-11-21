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
