import type { MachineConfig, ServerMessage, SystemAlias } from '#lib/generated/index.js';
import type { DeckConnection } from './deck-state.svelte.js';
import { EditQueue, type Edit, type EditOptions, type EditOutcome } from './edit-queue.svelte.js';
import type { Problem } from './problems.js';

const MACHINE_UNKNOWN: Problem = {
	text: 'Modification non enregistrée : la configuration de la machine n’est pas encore chargée.',
	detail: null
};

/**
 * What the UI knows about the machine: the shell aliases it manages (edited like the other
 * configurations, see {@link EditQueue}) and those the manager found in the startup files of
 * the shell. The system is read again only when the page asks, by {@link watch} and at each
 * new connection while the page is shown.
 */
export class MachineState {
	/** Aliases of the startup files that HuBergamof does not manage, as of the last reading. */
	systemAliases: SystemAlias[] = $state([]);

	readonly #connection: DeckConnection;
	readonly #edits: EditQueue<MachineConfig>;
	#watchers = 0;

	constructor(connection: DeckConnection, report: (problem: Problem) => void) {
		this.#connection = connection;
		this.#edits = new EditQueue<MachineConfig>({
			send: (config) => connection.send({ type: 'updateMachine', config }),
			report,
			gone: MACHINE_UNKNOWN
		});
	}

	/** Configuration with the pending edits applied. */
	get config(): MachineConfig | undefined {
		return this.#edits.config;
	}

	/** Whether an update is waiting for the manager's answer. */
	get pending(): boolean {
		return this.#edits.pending;
	}

	/**
	 * Asks the manager to read the system now, and again at each new connection until the
	 * returned function is called (the page is closed).
	 */
	watch(): () => void {
		this.#watchers += 1;
		this.#refresh();
		return () => {
			this.#watchers -= 1;
		};
	}

	/** A new connection started: the system is read again if the page is shown. */
	reconnected(): void {
		if (this.#watchers > 0) this.#refresh();
	}

	/** Changes the configuration; resolves once saved or dropped. */
	edit(change: Edit<MachineConfig>, options: EditOptions = {}): Promise<EditOutcome> {
		return this.#edits.edit(change, options);
	}

	/** Drops what waits for an answer, failed with `problem`. */
	dropPending(problem: Problem, options: { silent?: boolean } = {}): void {
		this.#edits.dropPending(problem, options);
	}

	/** Takes the machine message; the others are not its business. */
	handle(message: ServerMessage): void {
		if (message.type !== 'machine') return;
		this.systemAliases = message.systemAliases;
		this.#edits.receive(message.config);
	}

	#refresh(): void {
		this.#connection.send({ type: 'refreshMachine' });
	}
}
