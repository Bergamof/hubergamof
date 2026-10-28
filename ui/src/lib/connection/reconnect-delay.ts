/** Delay before the first reconnection attempt. */
export const INITIAL_RECONNECT_DELAY_MS = 500;

/** Upper bound of the reconnection delay, so the UI recovers quickly once the manager is back. */
export const MAX_RECONNECT_DELAY_MS = 30_000;

const BACKOFF_FACTOR = 2;

/** Exponential backoff delay for the given zero-based attempt, capped at {@link MAX_RECONNECT_DELAY_MS}. */
export function reconnectDelay(attempt: number): number {
	return Math.min(INITIAL_RECONNECT_DELAY_MS * BACKOFF_FACTOR ** attempt, MAX_RECONNECT_DELAY_MS);
}
