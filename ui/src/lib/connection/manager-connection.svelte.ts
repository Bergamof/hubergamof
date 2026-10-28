import type { ClientMessage, ServerMessage } from '#lib/generated/index.js';
import { reconnectDelay } from './reconnect-delay.js';
import { parseServerMessage } from './server-message.js';

/** Interval between two keep-alive pings while the connection is open. */
export const PING_INTERVAL_MS = 15_000;

export type ConnectionStatus = 'connecting' | 'open' | 'closed';

/** The subset of the browser `WebSocket` used by the connection (eases test doubles). */
export interface SocketLike {
	onopen: ((event: Event) => void) | null;
	onmessage: ((event: MessageEvent) => void) | null;
	onclose: ((event: CloseEvent) => void) | null;
	onerror: ((event: Event) => void) | null;
	send(data: string): void;
	close(): void;
}

export type SocketFactory = (url: string) => SocketLike;

export type ServerMessageListener = (message: ServerMessage) => void;

export interface ManagerConnectionOptions {
	url: string;
	/** Injected in tests; defaults to the browser `WebSocket`. */
	createSocket?: SocketFactory;
}

const createBrowserSocket: SocketFactory = (url) => new WebSocket(url);

/**
 * Typed WebSocket client to the manager: sends {@link ClientMessage}s, dispatches parsed
 * {@link ServerMessage}s, keeps the link alive with pings and reconnects with a capped
 * exponential backoff until {@link disconnect} is called.
 */
export class ManagerConnection {
	status: ConnectionStatus = $state('closed');

	readonly #url: string;
	readonly #createSocket: SocketFactory;
	#listeners: readonly ServerMessageListener[] = [];
	#socket: SocketLike | undefined;
	#reconnectAttempt = 0;
	#reconnectTimer: ReturnType<typeof setTimeout> | undefined;
	#pingTimer: ReturnType<typeof setInterval> | undefined;
	#active = false;

	constructor({ url, createSocket = createBrowserSocket }: ManagerConnectionOptions) {
		this.#url = url;
		this.#createSocket = createSocket;
	}

	/** Starts connecting; the connection then stays up (reconnecting if needed) until {@link disconnect}. */
	connect(): void {
		if (this.#active) return;
		this.#active = true;
		this.#reconnectAttempt = 0;
		this.#openSocket();
	}

	/** Closes the connection for good and cancels any pending reconnection. */
	disconnect(): void {
		this.#active = false;
		clearTimeout(this.#reconnectTimer);
		this.#reconnectTimer = undefined;
		this.#releaseSocket()?.close();
		this.status = 'closed';
	}

	/** Sends a message if the connection is open; returns whether it was sent. */
	send(message: ClientMessage): boolean {
		if (this.status !== 'open' || !this.#socket) return false;
		this.#socket.send(JSON.stringify(message));
		return true;
	}

	/** Registers a listener for server messages; returns its unsubscribe function. */
	onMessage(listener: ServerMessageListener): () => void {
		this.#listeners = [...this.#listeners, listener];
		return () => {
			this.#listeners = this.#listeners.filter((registered) => registered !== listener);
		};
	}

	#openSocket(): void {
		this.status = 'connecting';
		let socket: SocketLike;
		try {
			socket = this.#createSocket(this.#url);
		} catch {
			// An unreachable or malformed endpoint must not stop the retry loop.
			this.#handleClose();
			return;
		}
		this.#socket = socket;
		socket.onopen = () => this.#handleOpen();
		socket.onmessage = (event) => this.#handleMessage(event.data);
		// No `onerror` handler: browsers always follow `error` with `close`, which drives the reconnection.
		socket.onclose = () => this.#handleClose();
	}

	#handleOpen(): void {
		this.status = 'open';
		this.#reconnectAttempt = 0;
		this.#pingTimer = setInterval(() => this.send({ type: 'ping' }), PING_INTERVAL_MS);
	}

	#handleMessage(raw: unknown): void {
		const message = parseServerMessage(raw);
		if (!message) return;
		for (const listener of this.#listeners) listener(message);
	}

	#handleClose(): void {
		this.#releaseSocket();
		this.status = 'closed';
		if (this.#active) this.#scheduleReconnect();
	}

	#scheduleReconnect(): void {
		const delay = reconnectDelay(this.#reconnectAttempt);
		this.#reconnectAttempt += 1;
		this.#reconnectTimer = setTimeout(() => {
			this.#reconnectTimer = undefined;
			this.#openSocket();
		}, delay);
	}

	/** Detaches the current socket so its late events are ignored, and returns it. */
	#releaseSocket(): SocketLike | undefined {
		clearInterval(this.#pingTimer);
		this.#pingTimer = undefined;
		const socket = this.#socket;
		this.#socket = undefined;
		if (socket) {
			socket.onopen = null;
			socket.onmessage = null;
			socket.onclose = null;
		}
		return socket;
	}
}
