import { afterEach, describe, expect, it, vi } from 'vitest';
import { EditQueue, UPDATE_TIMEOUT_MS } from './edit-queue.svelte.js';

interface Doc {
	revision: number;
	value: number;
}

const GONE = { text: 'gone', detail: null };

function setup() {
	const sent: Doc[] = [];
	const reported: string[] = [];
	let open = true;
	const queue = new EditQueue<Doc>({
		send: (config) => {
			if (!open) return false;
			sent.push(config);
			return true;
		},
		report: (problem) => reported.push(problem.text),
		gone: GONE
	});
	return { queue, sent, reported, close: () => (open = false) };
}

const set = (value: number) => (doc: Doc) => ({ ...doc, value });

afterEach(() => {
	vi.useRealTimers();
});

describe('EditQueue', () => {
	it('refuses an edit while nothing is saved, saying why', async () => {
		const { queue, reported } = setup();
		await expect(queue.edit(set(1))).resolves.toEqual({ saved: false, problem: GONE });
		expect(reported).toEqual(['gone']);
	});

	it('sends an edit on the saved configuration and shows it at once', () => {
		const { queue, sent } = setup();
		queue.receive({ revision: 4, value: 0 });
		void queue.edit(set(1));
		expect(sent).toEqual([{ revision: 4, value: 1 }]);
		expect(queue.pending).toBe(true);
		expect(queue.config).toEqual({ revision: 4, value: 1 });
	});

	it('groups the edits made meanwhile into the next update', async () => {
		const { queue, sent } = setup();
		queue.receive({ revision: 0, value: 0 });
		const first = queue.edit(set(1));
		const second = queue.edit(set(2));
		const third = queue.edit((doc) => ({ ...doc, value: doc.value + 10 }));
		queue.receive({ revision: 1, value: 1 });
		await expect(first).resolves.toEqual({ saved: true });
		expect(sent.at(-1)).toEqual({ revision: 1, value: 12 });
		queue.receive({ revision: 2, value: 12 });
		await expect(Promise.all([second, third])).resolves.toEqual([{ saved: true }, { saved: true }]);
		expect(queue.pending).toBe(false);
	});

	it('does not take the save of another client for the answer', () => {
		const { queue } = setup();
		queue.receive({ revision: 0, value: 0 });
		void queue.edit(set(1));
		queue.receive({ revision: 1, value: 5 });
		expect(queue.pending).toBe(true);
		queue.receive({ revision: 3, value: 1 });
		expect(queue.pending).toBe(true);
		expect(queue.saved).toEqual({ revision: 3, value: 1 });
	});

	it('fails an edit when the connection is closed, without waiting for an answer', async () => {
		vi.useFakeTimers();
		const { queue, close, reported } = setup();
		queue.receive({ revision: 0, value: 0 });
		close();
		await expect(queue.edit(set(1))).resolves.toMatchObject({ saved: false });
		expect(vi.getTimerCount()).toBe(0);
		expect(reported).toHaveLength(1);
	});

	it('gives up after the timeout, once, with one notice for a mixed batch', async () => {
		vi.useFakeTimers();
		const { queue, reported } = setup();
		queue.receive({ revision: 0, value: 0 });
		const quiet = queue.edit(set(1), { notify: false });
		const loud = queue.edit(set(2));
		const quietToo = queue.edit(set(3), { notify: false });
		vi.advanceTimersByTime(UPDATE_TIMEOUT_MS);
		await expect(Promise.all([quiet, loud, quietToo])).resolves.toEqual([
			expect.objectContaining({ saved: false }),
			expect.objectContaining({ saved: false }),
			expect.objectContaining({ saved: false })
		]);
		expect(reported).toHaveLength(1);
		expect(queue.pending).toBe(false);
	});

	it('drops nothing, and reports nothing, when nothing waits', () => {
		const { queue, reported } = setup();
		queue.dropPending({ text: 'x', detail: null });
		expect(reported).toEqual([]);
	});

	it('fails silently when asked to, leaving the notice to the caller', async () => {
		const { queue, reported } = setup();
		queue.receive({ revision: 0, value: 0 });
		const outcome = queue.edit(set(1));
		queue.dropPending({ text: 'x', detail: null }, { silent: true });
		await expect(outcome).resolves.toMatchObject({ saved: false });
		expect(reported).toEqual([]);
	});

	it('forgets the configuration and fails what waits when what it configures goes away', async () => {
		const { queue } = setup();
		queue.receive({ revision: 0, value: 0 });
		const outcome = queue.edit(set(1), { notify: false });
		queue.forget();
		await expect(outcome).resolves.toEqual({ saved: false, problem: GONE });
		expect(queue.saved).toBeUndefined();
		expect(queue.config).toBeUndefined();
	});
});
