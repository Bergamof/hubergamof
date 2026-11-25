import { describe, expect, it } from 'vitest';
import { spanClass } from './FormCell.svelte';

describe('spanClass', () => {
	it('gives the container-query class of each span, written out in full', () => {
		expect(spanClass(2)).toBe('@xl:col-span-2');
		expect(spanClass(5)).toBe('@xl:col-span-5');
		expect(spanClass(12)).toBe('@xl:col-span-12');
	});
});
