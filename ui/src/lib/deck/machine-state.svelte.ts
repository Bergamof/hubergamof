import type { MachineConfig, ServerMessage, SystemAlias } from '#lib/generated/index.js';
import type { DeckConnection } from './deck-state.svelte.js';
import type { Problem } from './problems.js';
import { WatchedConfigState } from './watched-config-state.svelte.js';

const MACHINE_UNKNOWN: Problem = {
	text: 'Modification non enregistrée : la configuration de la machine n’est pas encore chargée.',
	detail: null
};

/**
 * What the UI knows about the machine: the shell aliases it manages and those the manager found
 * in the startup files of the shell.
 */
export class MachineState extends WatchedConfigState<MachineConfig> {
	/** Aliases of the startup files that HuBergamof does not manage, as of the last reading. */
	systemAliases: SystemAlias[] = $state([]);

	constructor(connection: DeckConnection, report: (problem: Problem) => void) {
		super({
			connection,
			report,
			update: (config) => ({ type: 'updateMachine', config }),
			refresh: { type: 'refreshMachine' },
			unknown: MACHINE_UNKNOWN
		});
	}

	/** Takes the machine message; the others are not its business. */
	handle(message: ServerMessage): void {
		if (message.type !== 'machine') return;
		this.systemAliases = message.systemAliases;
		this.receive(message.config);
	}
}
