// The microphone and camera buttons of a call carry no state attribute: only
// their label says what a click does, in the language of Teams. A label
// outside these tables gives `null` (unknown), never a guess.

/** @param {string} label */
const normalized = (label) => label.trim().toLowerCase();

const MICROPHONE_ON = [/^couper le micro/, /^désactiver le micro/, /^mute/];
const MICROPHONE_OFF = [/^réactiver le micro/, /^activer le micro/, /^unmute/];
const CAMERA_ON = [/^désactiver la caméra/, /^turn (off (my )?camera|camera off)/];
const CAMERA_OFF = [/^activer la caméra/, /^turn (on (my )?camera|camera on)/];

/**
 * @param {string} label
 * @param {RegExp[]} patterns
 */
const matchesAny = (label, patterns) => patterns.some((pattern) => pattern.test(label));

/**
 * Whether the microphone is muted, from the label of its button.
 * @param {string | null | undefined} label
 * @returns {boolean | null}
 */
export function mutedFromLabel(label) {
	const text = normalized(label ?? '');
	if (matchesAny(text, MICROPHONE_OFF)) return true;
	if (matchesAny(text, MICROPHONE_ON)) return false;
	return null;
}

/**
 * Whether the camera is on, from the label of its button.
 * @param {string | null | undefined} label
 * @returns {boolean | null}
 */
export function cameraOnFromLabel(label) {
	const text = normalized(label ?? '');
	if (matchesAny(text, CAMERA_OFF)) return false;
	if (matchesAny(text, CAMERA_ON)) return true;
	return null;
}
