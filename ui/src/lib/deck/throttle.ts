export interface Throttled<T> {
	/** Calls the function now, or at the end of the current period with the latest value. */
	(value: T): void;
	/** Calls the function now with `value` and cancels the pending call. */
	flush(value: T): void;
	/** Cancels the pending call. */
	cancel(): void;
}

/** Calls `run` at most once every `periodMs`, always ending with the latest value. */
export function throttle<T>(run: (value: T) => void, periodMs: number): Throttled<T> {
	let timer: ReturnType<typeof setTimeout> | undefined;
	let pending: { value: T } | undefined;

	const startPeriod = () => {
		timer = setTimeout(() => {
			timer = undefined;
			if (!pending) return;
			const { value } = pending;
			pending = undefined;
			run(value);
			startPeriod();
		}, periodMs);
	};

	const throttled = (value: T) => {
		if (timer) {
			pending = { value };
			return;
		}
		run(value);
		startPeriod();
	};
	throttled.cancel = () => {
		clearTimeout(timer);
		timer = undefined;
		pending = undefined;
	};
	throttled.flush = (value: T) => {
		throttled.cancel();
		run(value);
	};
	return throttled;
}
