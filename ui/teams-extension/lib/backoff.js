const FIRST_DELAY_MS = 1000;
const MAX_DELAY_MS = 60_000;

/**
 * Wait before reconnecting after `failures` quick failures in a row: doubles
 * from one second, at most a minute.
 * @param {number} failures
 */
export function reconnectDelay(failures) {
	if (failures <= 0) return 0;
	return Math.min(FIRST_DELAY_MS * 2 ** (failures - 1), MAX_DELAY_MS);
}
