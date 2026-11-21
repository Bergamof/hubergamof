import { describe, expect, it } from 'vitest';
import type { EditOutcome } from './edit-queue.svelte.js';
import { SAVE_STATUS_TEXT, SaveTracker } from './save-tracker.svelte.js';

/** An edit whose outcome the test decides when to give. */
function pending() {
	let settle!: (outcome: EditOutcome) => void;
	const outcome = new Promise<EditOutcome>((resolve) => (settle = resolve));
	return { outcome, settle };
}

describe('SaveTracker', () => {
	it('says saving while the manager has not answered, then saved', async () => {
		const tracker = new SaveTracker();
		expect(tracker.status).toBe('idle');
		const edit = pending();
		const tracking = tracker.track(edit.outcome);
		expect(tracker.status).toBe('saving');
		edit.settle({ saved: true } as EditOutcome);
		await tracking;
		expect(tracker.status).toBe('saved');
		expect(tracker.spoken).toBe(SAVE_STATUS_TEXT.saved);
	});

	it('says refused when the manager refused the change', async () => {
		const tracker = new SaveTracker();
		await tracker.track(Promise.resolve({ saved: false } as EditOutcome));
		expect(tracker.status).toBe('refused');
		expect(tracker.spoken).toBe(SAVE_STATUS_TEXT.refused);
	});

	it('stays idle and silent until an edit is made', () => {
		const tracker = new SaveTracker();
		expect([tracker.status, tracker.spoken]).toEqual(['idle', '']);
	});
});
