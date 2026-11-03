import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ServerMessage } from '#lib/generated/index.js';
import {
	ManagerConnection,
	PING_INTERVAL_MS,
	type ServerMessageListener,
	type SocketLike
} from './manager-connection.svelte.js';
import { INITIAL_RECONNECT_DELAY_MS, MAX_RECONNECT_DELAY_MS } from './reconnect-delay.js';

const URL = 'ws://localhost/ws';

class FakeSocket implements SocketLike {
	onopen: SocketLike['onopen'] = null;
	onmessage: SocketLike['onmessage'] = null;
	onclose: SocketLike['onclose'] = null;
	onerror: SocketLike['onerror'] = null;
	readonly sent: string[] = [];
	closed = false;

	constructor(readonly url: string) {}

	send(data: string): void {
		this.sent.push(data);
	}

	close(): void {
		this.closed = true;
	}

	simulateOpen(): void {
		this.onopen?.(new Event('open'));
	}

	simulateMessage(data: unknown): void {
		this.onmessage?.(new MessageEvent('message', { data }));
	}

	simulateClose(): void {
		this.onclose?.(new Event('close') as CloseEvent);
	}
}

function setup() {
	const sockets: FakeSocket[] = [];
	const connection = new ManagerConnection({
		url: URL,
		createSocket: (url) => {
			const socket = new FakeSocket(url);
			sockets.push(socket);
			return socket;
		}
	});
	const lastSocket = () => {
		const socket = sockets.at(-1);
		if (!socket) throw new Error('no socket created');
		return socket;
	};
	return { connection, sockets, lastSocket };
}

function openConnection() {
	const context = setup();
	context.connection.connect();
	context.lastSocket().simulateOpen();
	return context;
}

describe('ManagerConnection', () => {
	beforeEach(() => {
		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
	});

	describe('connection lifecycle', () => {
		it('is closed before connecting', () => {
			const { connection, sockets } = setup();

			expect(connection.status).toBe('closed');
			expect(sockets).toHaveLength(0);
		});

		it('opens a socket on the given URL and reports connecting, then open', () => {
			const { connection, lastSocket } = setup();

			connection.connect();
			expect(connection.status).toBe('connecting');
			expect(lastSocket().url).toBe(URL);

			lastSocket().simulateOpen();
			expect(connection.status).toBe('open');
		});

		it('ignores a second connect while already active', () => {
			const { connection, sockets } = setup();

			connection.connect();
			connection.connect();

			expect(sockets).toHaveLength(1);
		});

		it('closes the socket and stays closed on disconnect', () => {
			const { connection, sockets, lastSocket } = openConnection();

			connection.disconnect();
			vi.advanceTimersByTime(MAX_RECONNECT_DELAY_MS);

			expect(connection.status).toBe('closed');
			expect(lastSocket().closed).toBe(true);
			expect(sockets).toHaveLength(1);
		});
	});

	describe('sending', () => {
		it('serialises client messages as JSON when open', () => {
			const { connection, lastSocket } = openConnection();

			const sent = connection.send({ type: 'selectDevice', serial: 'ABC123', liveState: false });

			expect(sent).toBe(true);
			expect(JSON.parse(lastSocket().sent[0])).toEqual({
				type: 'selectDevice',
				serial: 'ABC123',
				liveState: false
			});
		});

		it('refuses to send while not open', () => {
			const { connection, lastSocket } = setup();
			connection.connect();

			expect(connection.send({ type: 'ping' })).toBe(false);
			expect(lastSocket().sent).toHaveLength(0);
		});

		it('pings periodically while open', () => {
			const { lastSocket } = openConnection();

			vi.advanceTimersByTime(PING_INTERVAL_MS * 2);

			expect(lastSocket().sent.map((data) => JSON.parse(data))).toEqual([
				{ type: 'ping' },
				{ type: 'ping' }
			]);
		});

		it('stops pinging once disconnected', () => {
			const { connection, lastSocket } = openConnection();

			connection.disconnect();
			vi.advanceTimersByTime(PING_INTERVAL_MS * 2);

			expect(lastSocket().sent).toHaveLength(0);
		});
	});

	describe('receiving', () => {
		function listen(connection: ManagerConnection) {
			const listener = vi.fn<ServerMessageListener>();
			const unsubscribe = connection.onMessage(listener);
			return { listener, unsubscribe };
		}

		it('dispatches parsed server messages to listeners', () => {
			const { connection, lastSocket } = openConnection();
			const { listener } = listen(connection);
			const message: ServerMessage = { type: 'devices', devices: [] };

			lastSocket().simulateMessage(JSON.stringify(message));

			expect(listener).toHaveBeenCalledExactlyOnceWith(message);
		});

		it.each([
			['malformed JSON', '{"type": "devices"'],
			['a non-object JSON value', '42'],
			['an object without type', '{"devices": []}'],
			['an unknown type', '{"type": "selfDestruct"}'],
			['a binary frame', new ArrayBuffer(4)]
		])('ignores %s without throwing', (_label, data) => {
			const { connection, lastSocket } = openConnection();
			const { listener } = listen(connection);

			expect(() => lastSocket().simulateMessage(data)).not.toThrow();
			expect(listener).not.toHaveBeenCalled();
			expect(connection.status).toBe('open');
		});

		it('stops notifying a listener after it unsubscribes', () => {
			const { connection, lastSocket } = openConnection();
			const { listener, unsubscribe } = listen(connection);

			unsubscribe();
			lastSocket().simulateMessage(JSON.stringify({ type: 'pong' }));

			expect(listener).not.toHaveBeenCalled();
		});
	});

	describe('reconnection', () => {
		it('reconnects after the initial delay when the socket closes', () => {
			const { connection, sockets, lastSocket } = openConnection();

			lastSocket().simulateClose();
			expect(connection.status).toBe('closed');

			vi.advanceTimersByTime(INITIAL_RECONNECT_DELAY_MS - 1);
			expect(sockets).toHaveLength(1);

			vi.advanceTimersByTime(1);
			expect(sockets).toHaveLength(2);
			expect(connection.status).toBe('connecting');
		});

		it('doubles the delay after each failed attempt, up to the cap', () => {
			const { connection, sockets, lastSocket } = setup();
			connection.connect();

			const observedDelays: number[] = [];
			for (let attempt = 0; attempt < 10; attempt += 1) {
				const socketCount = sockets.length;
				lastSocket().simulateClose();
				const before = Date.now();
				while (sockets.length === socketCount) vi.advanceTimersByTime(INITIAL_RECONNECT_DELAY_MS);
				observedDelays.push(Date.now() - before);
			}

			expect(observedDelays).toEqual([
				500, 1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000, 30_000, 30_000
			]);
		});

		it('resets the delay once a connection succeeds', () => {
			const { connection, sockets, lastSocket } = setup();
			connection.connect();
			lastSocket().simulateClose();
			vi.advanceTimersByTime(INITIAL_RECONNECT_DELAY_MS);
			lastSocket().simulateClose();
			vi.advanceTimersByTime(INITIAL_RECONNECT_DELAY_MS * 2);

			lastSocket().simulateOpen();
			lastSocket().simulateClose();
			vi.advanceTimersByTime(INITIAL_RECONNECT_DELAY_MS);

			expect(sockets).toHaveLength(4);
		});

		it('keeps retrying when the socket cannot even be created', () => {
			let calls = 0;
			const connection = new ManagerConnection({
				url: URL,
				createSocket: (url) => {
					calls += 1;
					if (calls === 1) throw new SyntaxError('invalid URL');
					return new FakeSocket(url);
				}
			});

			connection.connect();
			expect(connection.status).toBe('closed');

			vi.advanceTimersByTime(INITIAL_RECONNECT_DELAY_MS);
			expect(calls).toBe(2);
			expect(connection.status).toBe('connecting');
		});

		it('ignores events from a socket replaced by a reconnection', () => {
			const { connection, lastSocket } = openConnection();
			const staleSocket = lastSocket();
			staleSocket.simulateClose();
			vi.advanceTimersByTime(INITIAL_RECONNECT_DELAY_MS);

			staleSocket.simulateOpen();

			expect(connection.status).toBe('connecting');
		});
	});
});
