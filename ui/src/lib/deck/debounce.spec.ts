import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { debounce } from './debounce.js';

const DELAY_MS = 1000;

describe('debounce', () => {
	beforeEach(() => {
		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
	});

	it('runs once the calls stop for the whole delay', () => {
		const run = vi.fn();
		const debounced = debounce(run, DELAY_MS);
		debounced();
		vi.advanceTimersByTime(DELAY_MS - 1);
		debounced();
		vi.advanceTimersByTime(DELAY_MS - 1);
		expect(run).not.toHaveBeenCalled();
		expect(debounced.pending).toBe(true);
		vi.advanceTimersByTime(1);
		expect(run).toHaveBeenCalledOnce();
		expect(debounced.pending).toBe(false);
	});

	it('runs at once when flushed, and not again at the end of the delay', () => {
		const run = vi.fn();
		const debounced = debounce(run, DELAY_MS);
		debounced();
		debounced.flush();
		expect(run).toHaveBeenCalledOnce();
		vi.advanceTimersByTime(DELAY_MS);
		expect(run).toHaveBeenCalledOnce();
		debounced.flush();
		expect(run).toHaveBeenCalledTimes(2);
	});

	it('flushes only a running wait', () => {
		const run = vi.fn();
		const debounced = debounce(run, DELAY_MS);
		debounced.flushPending();
		expect(run).not.toHaveBeenCalled();
		debounced();
		debounced.flushPending();
		expect(run).toHaveBeenCalledOnce();
		debounced.flushPending();
		expect(run).toHaveBeenCalledOnce();
	});

	it('never runs once cancelled', () => {
		const run = vi.fn();
		const debounced = debounce(run, DELAY_MS);
		debounced();
		debounced.cancel();
		expect(debounced.pending).toBe(false);
		vi.advanceTimersByTime(DELAY_MS);
		expect(run).not.toHaveBeenCalled();
	});
});
