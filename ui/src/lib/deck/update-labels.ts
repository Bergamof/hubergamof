const SECONDS_TO_MILLISECONDS = 1000;
const NARROW_NO_BREAK_SPACE = '\u202f';

const timeFormat = new Intl.DateTimeFormat('fr-FR', { hour: 'numeric', minute: '2-digit' });
const dayFormat = new Intl.DateTimeFormat('fr-FR', { day: 'numeric', month: 'long' });
const yearFormat = new Intl.DateTimeFormat('fr-FR', {
	day: 'numeric',
	month: 'long',
	year: 'numeric'
});

function sameDay(first: Date, second: Date): boolean {
	return first.toDateString() === second.toDateString();
}

/**
 * When the last check for a newer release ran, in words: « aujourd’hui à 9 h 14 », « le 12 novembre
 * à 9 h 14 », and the year too when it is not this one. `now` is a parameter so that the wording is
 * the same whatever the day it is run. The spaces cannot break the line.
 */
export function checkedLabel(checkedAtSeconds: number, now: Date = new Date()): string {
	const checked = new Date(checkedAtSeconds * SECONDS_TO_MILLISECONDS);
	const time = timeFormat
		.format(checked)
		.replace(':', `${NARROW_NO_BREAK_SPACE}h${NARROW_NO_BREAK_SPACE}`);
	if (sameDay(checked, now)) return `aujourd’hui à ${time}`;
	const format = checked.getFullYear() === now.getFullYear() ? dayFormat : yearFormat;
	return `le ${format.format(checked)} à ${time}`;
}
