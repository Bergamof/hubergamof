import { sameJson } from './json.js';
import type { Problem } from './problems.js';

/** A change of a configuration, applied to the latest one known. */
export type Edit<C> = (config: C) => C;

/** How an edit ended: saved by the manager, or dropped (and why). */
export type EditOutcome = { saved: true } | { saved: false; problem: Problem };

export interface EditOptions {
	/**
	 * Whether a failure is shown as a notice (default). Off for callers showing it
	 * themselves, next to what failed.
	 */
	notify?: boolean;
}

/** Longest wait for the manager to answer an update before giving it up. */
export const UPDATE_TIMEOUT_MS = 10_000;

const NOT_CONNECTED: Problem = {
	text: 'Modification non enregistrée : pas de connexion au gestionnaire.',
	detail: null
};
export const CONNECTION_LOST: Problem = {
	text: 'Modification non enregistrée : la connexion au gestionnaire a été perdue.',
	detail: null
};
const NO_ANSWER: Problem = {
	text: 'Modification abandonnée : le gestionnaire n’a pas répondu.',
	detail: null
};

/** What the queue needs from its owner. */
export interface EditQueueHost<C> {
	/** Sends `config` to the manager; false when the connection is not open. */
	send(config: C): boolean;
	/** Shows `problem` until dismissed. */
	report(problem: Problem): void;
	/** Why edits are dropped when what they configure went away. */
	gone: Problem;
}

interface Waiter {
	settle: (outcome: EditOutcome) => void;
	notify: boolean;
}

interface QueuedEdit<C> extends Waiter {
	change: Edit<C>;
}

interface Update<C> {
	/** Configuration sent, with the revision it replaces. */
	config: C;
	waiters: Waiter[];
	timer: ReturnType<typeof setTimeout>;
}

/** Whether `received` is the manager saving `sent`: same content, next revision. */
function isSaved<C extends { revision: number }>(received: C, sent: C): boolean {
	return (
		received.revision === sent.revision + 1 &&
		sameJson({ ...received, revision: 0 }, { ...sent, revision: 0 })
	);
}

/**
 * Edits of one configuration kept by the manager (a device, a mouse, the radial menu).
 *
 * Updates are serialised: one is in flight at a time and the next edits wait for its
 * answer, then go together on top of the configuration the manager saved, so that quick
 * successive changes (brightness slider) never carry an outdated revision. Errors are not
 * tied to requests: any error arriving while an update is in flight answers it, and the
 * waiting edits are dropped with it. A configuration saved by another client meanwhile
 * does not answer it (the manager then refuses it as stale). {@link config} shows the
 * pending edits right away.
 */
export class EditQueue<C extends { revision: number }> {
	/** Last configuration sent by the manager. */
	saved: C | undefined = $state();

	#inFlight: Update<C> | undefined = $state();
	#queued: QueuedEdit<C>[] = $state([]);
	readonly #host: EditQueueHost<C>;

	/** Configuration as displayed: the saved one with the pending edits applied. */
	readonly config: C | undefined = $derived.by(() => {
		const base = this.#inFlight?.config ?? this.saved;
		return base && this.#queued.reduce((config, queued) => queued.change(config), base);
	});

	constructor(host: EditQueueHost<C>) {
		this.#host = host;
	}

	/** Whether an update is waiting for the manager's answer. */
	get pending(): boolean {
		return this.#inFlight !== undefined;
	}

	/** Changes the configuration; resolves once saved or dropped. */
	edit(change: Edit<C>, { notify = true }: EditOptions = {}): Promise<EditOutcome> {
		return new Promise((settle) => {
			const waiter = { settle, notify };
			if (!this.saved) {
				this.#fail([waiter], this.#host.gone);
			} else if (this.#inFlight) {
				this.#queued = [...this.#queued, { ...waiter, change }];
			} else {
				this.#send(change(this.saved), [waiter]);
			}
		});
	}

	/** The manager sent the configuration: it answers the update in flight when it is its save. */
	receive(config: C): void {
		this.saved = config;
		const update = this.#inFlight;
		if (!update || !isSaved(config, update.config)) return;
		clearTimeout(update.timer);
		this.#inFlight = undefined;
		for (const waiter of update.waiters) waiter.settle({ saved: true });
		this.#sendQueued(config);
	}

	/** What the configuration is about went away: forgets it and drops the pending edits. */
	forget(): void {
		this.saved = undefined;
		this.dropPending(this.#host.gone);
	}

	/**
	 * Drops the update in flight and the queued edits, all failed with `problem`, shown as a
	 * notice unless `silent` (the caller shows it once for several queues).
	 */
	dropPending(problem: Problem, { silent = false }: { silent?: boolean } = {}): void {
		const waiters: Waiter[] = [...(this.#inFlight?.waiters ?? []), ...this.#queued];
		clearTimeout(this.#inFlight?.timer);
		this.#inFlight = undefined;
		this.#queued = [];
		this.#fail(waiters, problem, silent);
	}

	#sendQueued(base: C): void {
		const queued = this.#queued;
		if (queued.length === 0) return;
		this.#queued = [];
		this.#send(
			queued.reduce((config, edit) => edit.change(config), base),
			queued
		);
	}

	#send(config: C, waiters: Waiter[]): void {
		if (!this.#host.send(config)) {
			this.#fail(waiters, NOT_CONNECTED);
			return;
		}
		const timer = setTimeout(() => this.dropPending(NO_ANSWER), UPDATE_TIMEOUT_MS);
		this.#inFlight = { config, waiters, timer };
	}

	/** Settles the waiters as failed, with one notice if any of them wants it. */
	#fail(waiters: readonly Waiter[], problem: Problem, silent = false): void {
		for (const waiter of waiters) waiter.settle({ saved: false, problem });
		if (!silent && waiters.some((waiter) => waiter.notify)) this.#host.report(problem);
	}
}
