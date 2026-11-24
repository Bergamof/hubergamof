// The duration of a call as Teams writes it ("12:34", "1:02:03"): digits and
// colons only, so a text that is anything else gives `null` (unknown), never
// a guess.

const DURATION = /^(?:(\d{1,3}):)?(\d{1,2}):(\d{2})$/;
const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 3600;

/**
 * @param {string | null | undefined} text
 * @returns {number | null} The seconds the text stands for.
 */
export function secondsFromDuration(text) {
	const match = DURATION.exec((text ?? '').trim());
	if (!match) return null;
	const [, hours, minutes, seconds] = match;
	const minutesPart = Number(minutes);
	const secondsPart = Number(seconds);
	if (minutesPart >= SECONDS_PER_MINUTE || secondsPart >= SECONDS_PER_MINUTE) return null;
	return Number(hours ?? 0) * SECONDS_PER_HOUR + minutesPart * SECONDS_PER_MINUTE + secondsPart;
}
