/**
 * Number of unread conversations announced by the window title, `(3) Chat |
 * Microsoft Teams`; `0` when the title carries no count.
 * @param {string} title
 */
export function unreadCountFromTitle(title) {
	const match = /^\((\d{1,4})\)/.exec(title.trim());
	return match ? Number(match[1]) : 0;
}
