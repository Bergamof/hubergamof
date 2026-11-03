import type { JsonValue } from '#lib/generated/index.js';

/** Deep equality of JSON values, regardless of the key order of objects. */
export function sameJson(left: JsonValue, right: JsonValue): boolean {
	if (left === right) return true;
	if (Array.isArray(left) || Array.isArray(right)) {
		return (
			Array.isArray(left) &&
			Array.isArray(right) &&
			left.length === right.length &&
			left.every((item, index) => sameJson(item, right[index]))
		);
	}
	if (typeof left !== 'object' || typeof right !== 'object' || left === null || right === null) {
		return false;
	}
	const keys = Object.keys(left);
	return (
		keys.length === Object.keys(right).length &&
		keys.every((key) => Object.hasOwn(right, key) && sameJson(left[key], right[key]))
	);
}

/**
 * Plain deep copy of a JSON value. Unlike `structuredClone`, it accepts the reactive
 * proxies of Svelte state; `$state.snapshot` would too, but its type explodes on the
 * recursive `JsonValue` (« Type instantiation is excessively deep »).
 */
export function cloneJson<T extends JsonValue>(value: T): T {
	return JSON.parse(JSON.stringify(value)) as T;
}
