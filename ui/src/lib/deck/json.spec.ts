import { describe, expect, it } from 'vitest';
import { cloneJson, sameJson } from './json.js';

describe('sameJson', () => {
	it('ignores the key order but not the values', () => {
		expect(sameJson({ a: 1, b: [1, { c: null }] }, { b: [1, { c: null }], a: 1 })).toBe(true);
		expect(sameJson({ a: 1 }, { a: 1, b: 2 })).toBe(false);
		expect(sameJson({ a: 1, b: 2 }, { a: 1, c: 2 })).toBe(false);
		expect(sameJson([1, 2], [2, 1])).toBe(false);
		expect(sameJson([1], { 0: 1 })).toBe(false);
		expect(sameJson(null, {})).toBe(false);
		expect(sameJson('1', 1)).toBe(false);
	});
});

describe('cloneJson', () => {
	it('copies deeply', () => {
		const original = { list: [1, { nested: true }] };
		const copy = cloneJson(original);
		expect(copy).toEqual(original);
		expect(copy.list).not.toBe(original.list);
	});
});
