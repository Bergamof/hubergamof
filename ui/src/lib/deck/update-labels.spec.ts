import { describe, expect, it } from 'vitest';
import { checkedLabel } from './update-labels.js';

const seconds = (date: Date) => Math.floor(date.getTime() / 1000);

describe('checkedLabel', () => {
	it('says today for a check of the same day', () => {
		const now = new Date(2026, 10, 25, 18, 0);
		expect(checkedLabel(seconds(new Date(2026, 10, 25, 9, 14)), now)).toBe('aujourd’hui à 09 h 14');
	});

	it('names the day of an older check', () => {
		const now = new Date(2026, 10, 25, 18, 0);
		expect(checkedLabel(seconds(new Date(2026, 10, 12, 7, 5)), now)).toBe(
			'le 12 novembre à 07 h 05'
		);
	});
});
