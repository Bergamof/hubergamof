import { describe, expect, it } from 'vitest';
import type { ServerMessage, SshConfig, SshSystemHost } from '#lib/generated/index.js';
import { DeckState } from './deck-state.svelte.js';
import { FakeConnection } from './fake-connection.js';
import { addHost } from './ssh-edits.js';

const FOUND: SshSystemHost[] = [
	{
		alias: 'web',
		hostName: 'web.example.org',
		user: '',
		port: null,
		identityFile: '',
		proxyJump: '',
		forwardAgent: false,
		otherOptions: []
	}
];

function ssh(revision: number): SshConfig {
	return { schemaVersion: 1, hosts: [], commonOptions: [], revision };
}

function setup() {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	return { connection, state: deck.ssh };
}

const HELLO: ServerMessage = { type: 'hello', version: '1.0.0' };
const NEW_HOST = {
	alias: 'prod',
	hostName: '',
	user: '',
	port: null,
	identityFile: '',
	proxyJump: '',
	forwardAgent: false,
	enabled: true,
	options: []
};

describe('SshState', () => {
	it('knows nothing before the manager has said, then takes what it says', () => {
		const { connection, state } = setup();
		expect(state.config).toBeUndefined();
		connection.receive({ type: 'ssh', config: ssh(1), systemHosts: FOUND });
		expect(state.config?.revision).toBe(1);
		expect(state.systemHosts).toEqual(FOUND);
	});

	it('reads the computer when watched and at each new connection while watched', () => {
		const { connection, state } = setup();
		const stop = state.watch();
		expect(connection.sent).toEqual([{ type: 'refreshSsh' }]);
		connection.receive(HELLO);
		expect(connection.sent.filter(({ type }) => type === 'refreshSsh')).toHaveLength(2);
		stop();
		connection.receive(HELLO);
		expect(connection.sent.filter(({ type }) => type === 'refreshSsh')).toHaveLength(2);
	});

	it('shows an edit at once and sends the whole configuration on its revision', () => {
		const { connection, state } = setup();
		connection.receive({ type: 'ssh', config: ssh(4), systemHosts: [] });
		void state.edit(addHost(NEW_HOST));
		expect(state.config?.hosts).toHaveLength(1);
		expect(connection.lastSent()).toMatchObject({
			type: 'updateSsh',
			config: { revision: 4, hosts: [{ alias: 'prod' }] }
		});
	});

	it('resolves an edit as saved once the manager sends the next revision', async () => {
		const { connection, state } = setup();
		connection.receive({ type: 'ssh', config: ssh(1), systemHosts: [] });
		const outcome = state.edit(addHost(NEW_HOST));
		connection.receive({
			type: 'ssh',
			config: { ...ssh(2), hosts: [NEW_HOST] },
			systemHosts: FOUND
		});
		await expect(outcome).resolves.toEqual({ saved: true });
	});

	it('refuses an edit while the configuration is unknown, with a notice', async () => {
		const connection = new FakeConnection();
		const deck = new DeckState(connection, { load: () => null, save: () => undefined });
		deck.listen();
		const outcome = await deck.ssh.edit(addHost(NEW_HOST));
		expect(outcome.saved).toBe(false);
		expect(deck.notices[0].text).toMatch(/SSH n’est pas encore chargée/);
	});

	it('drops what waits when the connection starts again', async () => {
		const { connection, state } = setup();
		connection.receive({ type: 'ssh', config: ssh(1), systemHosts: [] });
		const outcome = state.edit(addHost(NEW_HOST));
		connection.receive(HELLO);
		await expect(outcome).resolves.toMatchObject({ saved: false });
	});
});
