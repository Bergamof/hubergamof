import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { throttle } from './throttle.js';

const PERIOD_MS = 150;

describe('throttle', () => {
	beforeEach(() => {
		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
	});

	it('runs at once, then once per period with the latest value', () => {
		const calls: number[] = [];
		const throttled = throttle((value: number) => calls.push(value), PERIOD_MS);
		throttled(1);
		throttled(2);
		throttled(3);
		expect(calls).toEqual([1]);
		vi.advanceTimersByTime(PERIOD_MS);
		expect(calls).toEqual([1, 3]);
		vi.advanceTimersByTime(PERIOD_MS);
		expect(calls).toEqual([1, 3]);
		throttled(4);
		expect(calls).toEqual([1, 3, 4]);
	});

	it('flushes a final value right away and drops the pending one', () => {
		const calls: number[] = [];
		const throttled = throttle((value: number) => calls.push(value), PERIOD_MS);
		throttled(1);
		throttled(2);
		throttled.flush(5);
		vi.advanceTimersByTime(PERIOD_MS * 2);
		expect(calls).toEqual([1, 5]);
	});

	it('cancels the pending call', () => {
		const calls: number[] = [];
		const throttled = throttle((value: number) => calls.push(value), PERIOD_MS);
		throttled(1);
		throttled(2);
		throttled.cancel();
		vi.advanceTimersByTime(PERIOD_MS);
		expect(calls).toEqual([1]);
	});
});
