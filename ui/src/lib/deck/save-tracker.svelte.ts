import { tick } from 'svelte';
import type { EditOutcome } from './deck-state.svelte.js';

/** Where the last change stands: waiting for the manager, saved, or refused (a notice says why). */
export type SaveStatus = 'idle' | 'saving' | 'saved' | 'refused';

export const SAVE_STATUS_TEXT: Readonly<Record<SaveStatus, string>> = {
	idle: '',
	saving: 'Enregistrement…',
	saved: 'Modifications enregistrées.',
	refused: 'Dernière modification refusée.'
};

/** Follows the edits of a page and words them for the sighted (`status`) and for screen readers (`spoken`). */
export class SaveTracker {
	status: SaveStatus = $state('idle');
	/** Emptied first so that a repeated message is read again. */
	spoken = $state('');

	async track(outcome: Promise<EditOutcome>): Promise<void> {
		this.status = 'saving';
		const { saved } = await outcome;
		this.status = saved ? 'saved' : 'refused';
		this.spoken = '';
		await tick();
		this.spoken = SAVE_STATUS_TEXT[this.status];
	}
}
