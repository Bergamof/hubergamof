// The calendar (an Outlook frame) labels each slot "subject, 10:00 à 11:30,
// lundi 17 novembre 2026, organizer…" (French) or "subject, 10:00 AM to 11:30
// AM, Monday, November 17, 2026, …" (English).

const MONTHS = {
	janvier: 0,
	février: 1,
	mars: 2,
	avril: 3,
	mai: 4,
	juin: 5,
	juillet: 6,
	août: 7,
	septembre: 8,
	octobre: 9,
	novembre: 10,
	décembre: 11,
	january: 0,
	february: 1,
	march: 2,
	april: 3,
	may: 4,
	june: 5,
	july: 6,
	august: 7,
	september: 8,
	october: 9,
	november: 10,
	december: 11
};

const MONTH_NAMES = Object.keys(MONTHS).join('|');
const CLOCK = String.raw`(\d{1,2}):(\d{2})(?:\s?([AP]M))?`;
const SLOT = new RegExp(`^(.+?),\\s*${CLOCK}\\s+(?:à|to|-)\\s+${CLOCK}\\s*,\\s*(.+)$`, 'iu');
const DAY_FIRST = new RegExp(`(\\d{1,2})\\s+(${MONTH_NAMES})\\s+(\\d{4})`, 'iu');
const MONTH_FIRST = new RegExp(`(${MONTH_NAMES})\\s+(\\d{1,2}),?\\s+(\\d{4})`, 'iu');

/**
 * @param {string} hours
 * @param {string | undefined} meridiem
 */
function hourOfDay(hours, meridiem) {
	const hour = Number(hours);
	if (!meridiem) return hour;
	return (hour % 12) + (meridiem.toUpperCase() === 'PM' ? 12 : 0);
}

/** @param {string} text */
function dayOf(text) {
	const dayFirst = DAY_FIRST.exec(text);
	if (dayFirst) return { day: Number(dayFirst[1]), month: dayFirst[2], year: Number(dayFirst[3]) };
	const monthFirst = MONTH_FIRST.exec(text);
	if (monthFirst) {
		return { day: Number(monthFirst[2]), month: monthFirst[1], year: Number(monthFirst[3]) };
	}
	return null;
}

/**
 * The meeting a calendar slot describes, with its times in seconds since the
 * Unix epoch (local time of the browser, which is the computer's); `null`
 * for a label that is not a meeting (busy block, no subject, other format).
 * @param {string} label
 * @returns {{ subject: string, startsAt: number, endsAt: number } | null}
 */
export function meetingFromLabel(label) {
	const slot = SLOT.exec(label.trim());
	if (!slot) return null;
	const [
		,
		subject,
		startHours,
		startMinutes,
		startMeridiem,
		endHours,
		endMinutes,
		endMeridiem,
		rest
	] = slot;
	const day = dayOf(rest);
	if (!day || !subject.trim()) return null;
	const month = MONTHS[/** @type {keyof typeof MONTHS} */ (day.month.toLowerCase())];
	const start = new Date(
		day.year,
		month,
		day.day,
		hourOfDay(startHours, startMeridiem),
		Number(startMinutes)
	);
	const end = new Date(
		day.year,
		month,
		day.day,
		hourOfDay(endHours, endMeridiem),
		Number(endMinutes)
	);
	if (end <= start) end.setDate(end.getDate() + 1);
	return {
		subject: subject.trim(),
		startsAt: Math.floor(start.getTime() / 1000),
		endsAt: Math.floor(end.getTime() / 1000)
	};
}
