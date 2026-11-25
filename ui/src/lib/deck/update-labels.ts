const SECONDS_TO_MILLISECONDS = 1000;

const timeFormat = new Intl.DateTimeFormat('fr-FR', { hour: '2-digit', minute: '2-digit' });
const dayFormat = new Intl.DateTimeFormat('fr-FR', { day: 'numeric', month: 'long' });

function sameDay(first: Date, second: Date): boolean {
	return first.toDateString() === second.toDateString();
}

/**
 * When the last check for a newer release ran, in words: « aujourd’hui à 09:14 » or « le 12 novembre
 * à 09:14 ». `now` is a parameter so that the wording is the same whatever the day it is run.
 */
export function checkedLabel(checkedAtSeconds: number, now: Date = new Date()): string {
	const checked = new Date(checkedAtSeconds * SECONDS_TO_MILLISECONDS);
	const time = timeFormat.format(checked).replace(':', ' h ');
	return sameDay(checked, now)
		? `aujourd’hui à ${time}`
		: `le ${dayFormat.format(checked)} à ${time}`;
}
