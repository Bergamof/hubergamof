import { tick } from 'svelte';
import type { EditOutcome } from './edit-queue.svelte.js';
import type { JsonValue } from '#lib/generated/index.js';
import { sameJson } from './json.js';

/**
 * What a form that is sent as a whole shows and does: the saved values until something is typed,
 * then the draft; the note « not saved yet » only while the draft differs from what is saved; and
 * the sending, which checks first and puts the focus in the first field that is wrong.
 */
export class FormDraft<T extends Record<string, JsonValue>> {
	/** What was typed since the last save; none: the saved values are shown. */
	draft: T | undefined = $state();
	/** The form was sent once: its mistakes are said under the fields. */
	submitted = $state(false);

	readonly #saved: () => T;

	/** `saved` gives the values the manager holds, read each time (they change under the form). */
	constructor(saved: () => T) {
		this.#saved = saved;
	}

	get shown(): T {
		return this.draft ?? this.#saved();
	}

	/** Something typed that is not what is saved. */
	get dirty(): boolean {
		return this.draft !== undefined && !sameJson(this.draft, this.#saved());
	}

	/** Changes some of the values shown. */
	edit(patch: Partial<T>): void {
		this.draft = { ...this.shown, ...patch };
	}

	/** Gives up what was typed. */
	reset(): void {
		this.draft = undefined;
		this.submitted = false;
	}

	/**
	 * Sends the form. `prepare` makes what is sent of the values (trimmed…); `problem` gives, for
	 * what is sent, how to put the focus in the first field that is wrong, if one is; `send`
	 * saves. Once saved the form shows the saved values again.
	 */
	async submit(
		event: SubmitEvent,
		prepare: (values: T) => T,
		problem: (values: T) => (() => void) | undefined,
		send: (values: T) => Promise<EditOutcome>
	): Promise<boolean> {
		event.preventDefault();
		this.submitted = true;
		const values = prepare(this.shown);
		const focusProblem = problem(values);
		if (focusProblem) {
			await tick();
			focusProblem();
			return false;
		}
		const { saved } = await send(values);
		if (saved) this.reset();
		return saved;
	}
}
