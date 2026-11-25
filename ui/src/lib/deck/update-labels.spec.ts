import { describe, expect, it } from 'vitest';
import { checkedLabel } from './update-labels.js';

const seconds = (date: Date) => Math.floor(date.getTime() / 1000);
const NOW = new Date(2026, 10, 25, 18, 0);
const NBSP = '\u202f';

describe('checkedLabel', () => {
	it('says today for a check of the same day', () => {
		expect(checkedLabel(seconds(new Date(2026, 10, 25, 9, 14)), NOW)).toBe(
			`aujourd’hui à 9${NBSP}h${NBSP}14`
		);
	});

	it('names the day of an older check', () => {
		expect(checkedLabel(seconds(new Date(2026, 10, 12, 7, 5)), NOW)).toBe(
			`le 12 novembre à 7${NBSP}h${NBSP}05`
		);
	});

	it('adds the year when it is not this one', () => {
		expect(checkedLabel(seconds(new Date(2025, 10, 12, 7, 5)), NOW)).toBe(
			`le 12 novembre 2025 à 7${NBSP}h${NBSP}05`
		);
	});
});
