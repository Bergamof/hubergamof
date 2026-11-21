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
