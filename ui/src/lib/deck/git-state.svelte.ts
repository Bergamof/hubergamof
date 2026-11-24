import type { GitConfig, GitSystemEntry, ServerMessage } from '#lib/generated/index.js';
import type { DeckConnection } from './deck-state.svelte.js';
import type { Problem } from './problems.js';
import { WatchedConfigState } from './watched-config-state.svelte.js';

const GIT_UNKNOWN: Problem = {
	text: 'Modification non enregistrée : la configuration de Git n’est pas encore chargée.',
	detail: null
};

/**
 * What the UI knows about Git: the configuration HuBergamof manages and the managed values the
 * manager found in other files of the computer.
 */
export class GitState extends WatchedConfigState<GitConfig> {
	/** Managed values that Git also finds elsewhere, as of the last reading. */
	systemEntries: GitSystemEntry[] = $state([]);
	/** Whether the manager could run `git` at the last reading; `undefined` before the first one. */
	gitAvailable: boolean | undefined = $state();

	constructor(connection: DeckConnection, report: (problem: Problem) => void) {
		super({
			connection,
			report,
			update: (config) => ({ type: 'updateGit', config }),
			refresh: { type: 'refreshGit' },
			unknown: GIT_UNKNOWN
		});
	}

	/** Takes the git message; the others are not its business. */
	handle(message: ServerMessage): void {
		if (message.type !== 'git') return;
		this.systemEntries = message.systemEntries;
		this.gitAvailable = message.gitAvailable;
		this.receive(message.config);
	}
}
