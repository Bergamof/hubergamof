export interface Debounced {
	/** (Re)starts the wait: `run` is called once no new call came for the whole delay. */
	(): void;
	/** Calls `run` now, cancelling the wait. */
	flush(): void;
	/** Calls `run` now if a wait is running: nothing waits any more. */
	flushPending(): void;
	/** Cancels the wait. */
	cancel(): void;
	/** A wait is running. */
	readonly pending: boolean;
}

/** Calls `run` once the calls stop for `delayMs` (the end of a typing burst). */
export function debounce(run: () => void, delayMs: number): Debounced {
	let timer: ReturnType<typeof setTimeout> | undefined;

	const cancel = () => {
		clearTimeout(timer);
		timer = undefined;
	};
	const flush = () => {
		cancel();
		run();
	};
	const flushPending = () => {
		if (timer !== undefined) flush();
	};
	const debounced = () => {
		cancel();
		timer = setTimeout(flush, delayMs);
	};
	return Object.defineProperties(debounced, {
		flush: { value: flush },
		flushPending: { value: flushPending },
		cancel: { value: cancel },
		pending: { get: () => timer !== undefined }
	}) as Debounced;
}
