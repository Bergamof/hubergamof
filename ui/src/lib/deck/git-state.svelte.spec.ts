import { afterEach, describe, expect, it, vi } from 'vitest';
import type { GitSystemEntry, ServerMessage } from '#lib/generated/index.js';
import { DeckState, UPDATE_TIMEOUT_MS } from './deck-state.svelte.js';
import { FakeConnection } from './fake-connection.js';
import { addAlias } from './machine-edits.js';
import { emptyGit } from './git-test-support.js';
import { setIdentity } from './git-edits.js';

const FOUND: GitSystemEntry[] = [{ key: 'user.name', value: 'Ada', origin: '~/.gitconfig' }];

function setup() {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	return { connection, deck, state: deck.git };
}

const HELLO: ServerMessage = { type: 'hello', version: '1.0.0' };

function gitMessage(revision: number, found: GitSystemEntry[] = [], available = true) {
	return {
		type: 'git',
		config: emptyGit(revision),
		systemEntries: found,
		gitAvailable: available
	} satisfies ServerMessage;
}

afterEach(() => vi.useRealTimers());

describe('GitState', () => {
	it('knows nothing before the manager has said', () => {
		const { state } = setup();
		expect(state.config).toBeUndefined();
		expect(state.systemEntries).toEqual([]);
		expect(state.gitAvailable).toBeUndefined();
	});

	it('takes the configuration, the values found elsewhere and whether git ran', () => {
		const { connection, state } = setup();
		connection.receive(gitMessage(1, FOUND, false));
		expect(state.config?.revision).toBe(1);
		expect(state.systemEntries).toEqual(FOUND);
		expect(state.gitAvailable).toBe(false);
	});

	it('asks to read the computer when watched, and at each new connection while watched', () => {
		const { connection, state } = setup();
		const stop = state.watch();
		expect(connection.sent).toEqual([{ type: 'refreshGit' }]);

		connection.receive(HELLO);
		expect(connection.sent.filter(({ type }) => type === 'refreshGit')).toHaveLength(2);

		stop();
		connection.receive(HELLO);
		expect(connection.sent.filter(({ type }) => type === 'refreshGit')).toHaveLength(2);
	});

	it('does not read the computer on a connection when nobody watches', () => {
		const { connection } = setup();
		connection.receive(HELLO);
		expect(connection.sent).toEqual([]);
	});

	it('shows an edit at once and sends the whole configuration on its revision', () => {
		const { connection, state } = setup();
		connection.receive(gitMessage(4));
		void state.edit(setIdentity({ name: 'Ada', email: 'ada@example.org' }));
		expect(state.pending).toBe(true);
		expect(state.config?.identity.name).toBe('Ada');
		const sent = connection.lastSent();
		expect(sent).toMatchObject({ type: 'updateGit', config: { revision: 4 } });
		expect(sent).toMatchObject({ config: { identity: { email: 'ada@example.org' } } });
	});

	it('resolves an edit as saved once the manager sends the next revision', async () => {
		const { connection, state } = setup();
		connection.receive(gitMessage(1));
		const outcome = state.edit(setIdentity({ name: 'Ada', email: '' }));
		const saved = emptyGit(2);
		saved.identity.name = 'Ada';
		connection.receive({ type: 'git', config: saved, systemEntries: FOUND, gitAvailable: true });
		await expect(outcome).resolves.toEqual({ saved: true });
		expect(state.pending).toBe(false);
	});

	it('refuses an edit while the configuration is unknown, with a notice', async () => {
		const { deck, state } = setup();
		const outcome = await state.edit(setIdentity({ name: 'Ada', email: '' }));
		expect(outcome.saved).toBe(false);
		expect(deck.notices[0].text).toMatch(/Git n’est pas encore chargée/);
	});

	it('answers the update in flight with the error of the manager', async () => {
		const { connection, deck, state } = setup();
		connection.receive(gitMessage(1));
		const outcome = state.edit(addAlias({ name: 'co', command: 'checkout', enabled: true }));
		connection.receive({ type: 'error', code: 'invalidConfig', message: 'invalid' });
		await expect(outcome).resolves.toMatchObject({ saved: false });
		expect(deck.notices).toHaveLength(1);
		expect(state.config?.aliases).toEqual([]);
	});

	it('shows one notice when the machine and Git were both waiting', async () => {
		const { connection, deck, state } = setup();
		connection.receive(gitMessage(1));
		connection.receive({
			type: 'machine',
			config: { schemaVersion: 1, aliases: [], revision: 1 },
			systemAliases: []
		});
		const git = state.edit(setIdentity({ name: 'Ada', email: '' }));
		const machine = deck.machine.edit(addAlias({ name: 'll', command: 'ls', enabled: true }));
		connection.receive({ type: 'error', code: 'locked', message: 'locked' });
		await Promise.all([git, machine]);
		expect(deck.notices).toHaveLength(1);
	});

	it('gives up an edit the manager does not answer', async () => {
		vi.useFakeTimers();
		const { connection, state } = setup();
		connection.receive(gitMessage(1));
		const outcome = state.edit(setIdentity({ name: 'Ada', email: '' }));
		vi.advanceTimersByTime(UPDATE_TIMEOUT_MS);
		await expect(outcome).resolves.toMatchObject({ saved: false });
	});

	it('drops what waits when the connection starts again', async () => {
		const { connection, state } = setup();
		connection.receive(gitMessage(1));
		const outcome = state.edit(setIdentity({ name: 'Ada', email: '' }));
		connection.receive(HELLO);
		await expect(outcome).resolves.toMatchObject({ saved: false });
	});
});
