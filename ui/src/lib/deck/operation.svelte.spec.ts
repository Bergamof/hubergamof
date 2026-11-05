import { describe, expect, it } from 'vitest';
import { Operation } from './operation.svelte.js';

const PROBLEM = { text: 'Refusé.', detail: null };

describe('Operation', () => {
	it('is busy while running, then shows its status', async () => {
		const operation = new Operation();
		let finish: (problem: undefined) => void = () => {};
		const running = operation.run(() => new Promise((resolve) => (finish = resolve)), 'Fait.');
		expect(operation.busy).toBe(true);
		expect(operation.status).toBe('');
		finish(undefined);
		await expect(running).resolves.toBe(true);
		expect(operation.busy).toBe(false);
		expect(operation.status).toBe('Fait.');
		expect(operation.problem).toBeUndefined();
	});

	it('keeps the problem of a failed run and forgets it on the next one', async () => {
		const operation = new Operation();
		await expect(operation.run(async () => PROBLEM, 'Fait.')).resolves.toBe(false);
		expect(operation.problem).toEqual(PROBLEM);
		expect(operation.status).toBe('');
		await operation.run(async () => undefined, 'Fait.');
		expect(operation.problem).toBeUndefined();
		operation.clear();
		expect(operation.status).toBe('');
	});

	it('ignores a run started while another is under way', async () => {
		const operation = new Operation();
		let calls = 0;
		let finish: (problem: undefined) => void = () => {};
		const first = operation.run(() => {
			calls += 1;
			return new Promise((resolve) => (finish = resolve));
		}, 'Fait.');
		await expect(operation.run(async () => undefined, 'Autre.')).resolves.toBe(false);
		finish(undefined);
		await first;
		expect(calls).toBe(1);
		expect(operation.status).toBe('Fait.');
	});
});
