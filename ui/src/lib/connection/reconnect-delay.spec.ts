import { describe, expect, it } from 'vitest';
import {
	INITIAL_RECONNECT_DELAY_MS,
	MAX_RECONNECT_DELAY_MS,
	reconnectDelay
} from './reconnect-delay.js';

describe('reconnectDelay', () => {
	it('starts at the initial delay', () => {
		expect(reconnectDelay(0)).toBe(INITIAL_RECONNECT_DELAY_MS);
	});

	it('doubles with each attempt', () => {
		expect(reconnectDelay(3)).toBe(INITIAL_RECONNECT_DELAY_MS * 8);
	});

	it('never exceeds the cap', () => {
		expect(reconnectDelay(1_000)).toBe(MAX_RECONNECT_DELAY_MS);
	});
});
