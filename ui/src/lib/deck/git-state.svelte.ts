import type { GitConfig, GitSystemEntry, ServerMessage } from '#lib/generated/index.js';
import type { DeckConnection } from './deck-state.svelte.js';
import { EditQueue, type Edit, type EditOptions, type EditOutcome } from './edit-queue.svelte.js';
import type { Problem } from './problems.js';

const GIT_UNKNOWN: Problem = {
	text: 'Modification non enregistrée : la configuration de Git n’est pas encore chargée.',
	detail: null
};

/**
 * What the UI knows about Git: the configuration HuBergamof manages (edited like the other
 * configurations, see {@link EditQueue}) and the managed values the manager found in other files
 * of the computer. The computer is read again only when the page asks, by {@link watch} and at
 * each new connection while the page is shown.
 */
export class GitState {
	/** Managed values that Git also finds elsewhere, as of the last reading. */
	systemEntries: GitSystemEntry[] = $state([]);
	/** Whether the manager could run `git` at the last reading; `undefined` before the first one. */
	gitAvailable: boolean | undefined = $state();

	readonly #connection: DeckConnection;
	readonly #edits: EditQueue<GitConfig>;
	#watchers = 0;

	constructor(connection: DeckConnection, report: (problem: Problem) => void) {
		this.#connection = connection;
		this.#edits = new EditQueue<GitConfig>({
			send: (config) => connection.send({ type: 'updateGit', config }),
			report,
			gone: GIT_UNKNOWN
		});
	}

	/** Configuration with the pending edits applied. */
	get config(): GitConfig | undefined {
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
		this.#refresh();
		return () => {
			this.#watchers -= 1;
		};
	}

	/** A new connection started: the computer is read again if the page is shown. */
	reconnected(): void {
		if (this.#watchers > 0) this.#refresh();
	}

	/** Changes the configuration; resolves once saved or dropped. */
	edit(change: Edit<GitConfig>, options: EditOptions = {}): Promise<EditOutcome> {
		return this.#edits.edit(change, options);
	}

	/** Drops what waits for an answer, failed with `problem`. */
	dropPending(problem: Problem, options: { silent?: boolean } = {}): void {
		this.#edits.dropPending(problem, options);
	}

	/** Takes the git message; the others are not its business. */
	handle(message: ServerMessage): void {
		if (message.type !== 'git') return;
		this.systemEntries = message.systemEntries;
		this.gitAvailable = message.gitAvailable;
		this.#edits.receive(message.config);
	}

	#refresh(): void {
		this.#connection.send({ type: 'refreshGit' });
	}
}
