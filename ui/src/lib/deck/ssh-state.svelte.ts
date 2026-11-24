import type { ServerMessage, SshConfig, SshSystemHost } from '#lib/generated/index.js';
import type { DeckConnection } from './deck-state.svelte.js';
import type { Problem } from './problems.js';
import { WatchedConfigState } from './watched-config-state.svelte.js';

const SSH_UNKNOWN: Problem = {
	text: 'Modification non enregistrée : la configuration de SSH n’est pas encore chargée.',
	detail: null
};

/**
 * What the UI knows about SSH: the hosts HuBergamof manages and those the manager found in
 * `~/.ssh/config` that it does not manage.
 */
export class SshState extends WatchedConfigState<SshConfig> {
	/** Hosts of `~/.ssh/config` that HuBergamof does not manage, as of the last reading. */
	systemHosts: SshSystemHost[] = $state([]);

	constructor(connection: DeckConnection, report: (problem: Problem) => void) {
		super({
			connection,
			report,
			update: (config) => ({ type: 'updateSsh', config }),
			refresh: { type: 'refreshSsh' },
			unknown: SSH_UNKNOWN
		});
	}

	/** Takes the ssh message; the others are not its business. */
	handle(message: ServerMessage): void {
		if (message.type !== 'ssh') return;
		this.systemHosts = message.systemHosts;
		this.receive(message.config);
	}
}
