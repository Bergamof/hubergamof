import { afterEach, describe, expect, it, vi } from 'vitest';
import type { MachineConfig, ServerMessage, SystemAlias } from '#lib/generated/index.js';
import { DeckState, UPDATE_TIMEOUT_MS } from './deck-state.svelte.js';
import { FakeConnection } from './fake-connection.js';
import { addAlias } from './machine-edits.js';

const SYSTEM: SystemAlias[] = [{ name: 'gs', command: 'git status', source: '.bashrc' }];

function machine(revision: number, names: string[] = []): MachineConfig {
	return {
		schemaVersion: 1,
		aliases: names.map((name) => ({ name, command: 'ls', enabled: true })),
		revision
	};
}

function setup() {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	return { connection, deck, state: deck.machine };
}

const HELLO: ServerMessage = { type: 'hello', version: '1.0.0' };

afterEach(() => vi.useRealTimers());

describe('MachineState', () => {
	it('knows nothing before the manager has said', () => {
		const { state } = setup();
		expect(state.config).toBeUndefined();
		expect(state.systemAliases).toEqual([]);
	});

	it('takes the configuration and the aliases of the system', () => {
		const { connection, state } = setup();
		connection.receive({ type: 'machine', config: machine(1, ['ll']), systemAliases: SYSTEM });
		expect(state.config?.aliases[0].name).toBe('ll');
		expect(state.systemAliases).toEqual(SYSTEM);
	});

	it('asks to read the system when watched, and at each new connection while watched', () => {
		const { connection, state } = setup();
		const stop = state.watch();
		expect(connection.sent).toEqual([{ type: 'refreshMachine' }]);

		connection.receive(HELLO);
		expect(connection.sent.filter(({ type }) => type === 'refreshMachine')).toHaveLength(2);

		stop();
		connection.receive(HELLO);
		expect(connection.sent.filter(({ type }) => type === 'refreshMachine')).toHaveLength(2);
	});

	it('does not read the system on a connection when nobody watches', () => {
		const { connection } = setup();
		connection.receive(HELLO);
		expect(connection.sent).toEqual([]);
	});

	it('shows an edit at once and sends the whole configuration on its revision', () => {
		const { connection, state } = setup();
		connection.receive({ type: 'machine', config: machine(4), systemAliases: [] });
		void state.edit(addAlias({ name: 'll', command: 'ls -l', enabled: true }));
		expect(state.pending).toBe(true);
		expect(state.config?.aliases).toHaveLength(1);
		expect(connection.lastSent()).toEqual({
			type: 'updateMachine',
			config: {
				schemaVersion: 1,
				aliases: [{ name: 'll', command: 'ls -l', enabled: true }],
				revision: 4
			}
		});
	});

	it('resolves an edit as saved once the manager sends the next revision', async () => {
		const { connection, state } = setup();
		connection.receive({ type: 'machine', config: machine(1), systemAliases: [] });
		const outcome = state.edit(addAlias({ name: 'll', command: 'ls', enabled: true }));
		connection.receive({ type: 'machine', config: machine(2, ['ll']), systemAliases: SYSTEM });
		await expect(outcome).resolves.toEqual({ saved: true });
		expect(state.pending).toBe(false);
	});

	it('refuses an edit while the configuration is unknown, with a notice', async () => {
		const { deck, state } = setup();
		const outcome = await state.edit(addAlias({ name: 'll', command: 'ls', enabled: true }));
		expect(outcome.saved).toBe(false);
		expect(deck.notices[0].text).toMatch(/pas encore chargée/);
	});

	it('answers the update in flight with the error of the manager', async () => {
		const { connection, deck, state } = setup();
		connection.receive({ type: 'machine', config: machine(1), systemAliases: [] });
		const outcome = state.edit(addAlias({ name: 'll', command: 'ls', enabled: true }));
		connection.receive({ type: 'error', code: 'staleRevision', message: 'stale' });
		await expect(outcome).resolves.toMatchObject({ saved: false });
		expect(deck.notices).toHaveLength(1);
		expect(state.config?.aliases).toEqual([]);
	});

	it('shows one notice when the deck and the machine were both waiting', async () => {
		const { connection, deck, state } = setup();
		connection.receive({ type: 'machine', config: machine(1), systemAliases: [] });
		connection.receive({ type: 'devices', devices: [] });
		const outcome = state.edit(addAlias({ name: 'll', command: 'ls', enabled: true }));
		connection.receive({ type: 'error', code: 'locked', message: 'locked' });
		await outcome;
		expect(deck.notices).toHaveLength(1);
	});

	it('gives up an edit the manager does not answer', async () => {
		vi.useFakeTimers();
		const { connection, state } = setup();
		connection.receive({ type: 'machine', config: machine(1), systemAliases: [] });
		const outcome = state.edit(addAlias({ name: 'll', command: 'ls', enabled: true }));
		vi.advanceTimersByTime(UPDATE_TIMEOUT_MS);
		await expect(outcome).resolves.toMatchObject({ saved: false });
	});

	it('drops what waits when the connection starts again', async () => {
		const { connection, state } = setup();
		connection.receive({ type: 'machine', config: machine(1), systemAliases: [] });
		const outcome = state.edit(addAlias({ name: 'll', command: 'ls', enabled: true }));
		connection.receive(HELLO);
		await expect(outcome).resolves.toMatchObject({ saved: false });
	});
});
