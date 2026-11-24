import type { ClientMessage } from '#lib/generated/index.js';
import type { DeckConnection } from './deck-state.svelte.js';
import { EditQueue, type Edit, type EditOptions, type EditOutcome } from './edit-queue.svelte.js';
import type { Problem } from './problems.js';

/** What a configuration of the machine needs to talk to the manager. */
export interface WatchedConfigOptions<C> {
	connection: DeckConnection;
	report: (problem: Problem) => void;
	/** The message that saves a whole configuration. */
	update: (config: C) => ClientMessage;
	/** The message that asks the manager to read the computer. */
	refresh: ClientMessage;
	/** Why an edit is dropped while the configuration is not loaded. */
	unknown: Problem;
}

/**
 * A configuration of the machine (shell aliases, Git…): edited like the other configurations, see
 * {@link EditQueue}, and shown next to what the manager reads from the computer. The computer is
 * read again only when a page asks, by {@link watch} and at each new connection while one is
 * shown, never watched.
 */
export abstract class WatchedConfigState<C extends { revision: number }> {
	readonly #connection: DeckConnection;
	readonly #refresh: ClientMessage;
	readonly #edits: EditQueue<C>;
	#watchers = 0;

	protected constructor({ connection, report, update, refresh, unknown }: WatchedConfigOptions<C>) {
		this.#connection = connection;
		this.#refresh = refresh;
		this.#edits = new EditQueue<C>({
			send: (config) => connection.send(update(config)),
			report,
			gone: unknown
		});
	}

	/** Configuration with the pending edits applied. */
	get config(): C | undefined {
		return this.#edits.config;
	}

	/** Whether an update is waiting for the manager's answer. */
	get pending(): boolean {
		return this.#edits.pending;
	}

	/**
	 * Asks the manager to read the computer now, and again at each new connection until the
	 * returned function is called (the page is closed).
	 */
	watch(): () => void {
		this.#watchers += 1;
		this.#read();
		return () => {
			this.#watchers -= 1;
		};
	}

	/** A new connection started: the computer is read again if a page is shown. */
	reconnected(): void {
		if (this.#watchers > 0) this.#read();
	}

	/** Changes the configuration; resolves once saved or dropped. */
	edit(change: Edit<C>, options: EditOptions = {}): Promise<EditOutcome> {
		return this.#edits.edit(change, options);
	}

	/** Drops what waits for an answer, failed with `problem`. */
	dropPending(problem: Problem, options: { silent?: boolean } = {}): void {
		this.#edits.dropPending(problem, options);
	}

	/** The manager sent the saved configuration. */
	protected receive(config: C): void {
		this.#edits.receive(config);
	}

	#read(): void {
		this.#connection.send(this.#refresh);
	}
}
