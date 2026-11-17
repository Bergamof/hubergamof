import { reconnectDelay } from './backoff.js';

/** A connection that lasted this long was working: its end starts the backoff afresh. */
export const HEALTHY_AFTER_MS = 5000;

/**
 * Link between the tabs and the manager's native host. Keeps the latest report
 * of each type and replays them on every new connection (the manager forgets
 * everything when the link drops); reconnects with a growing delay while the
 * manager is absent.
 *
 * @typedef {{ postMessage: (message: unknown) => void,
 *   onMessage: { addListener: (listener: (message: unknown) => void) => void },
 *   onDisconnect: { addListener: (listener: () => void) => void } }} Port
 */
export class Relay {
	/** @type {Port | null} */
	#port = null;
	/** @type {Map<string, unknown>} */
	#latest = new Map();
	#failures = 0;
	#retryAt = 0;

	/**
	 * @param {{ connect: () => Port, now: () => number,
	 *   onRequest: (message: unknown) => void }} deps
	 */
	constructor(deps) {
		this.deps = deps;
	}

	/**
	 * Keeps `report` as the latest of its type and sends it if linked.
	 * @param {{ type: string }} report
	 */
	report(report) {
		this.#latest.set(report.type, report);
		// A new connection replays every latest report, this one included.
		if (this.#port) this.#port.postMessage(report);
		else this.connect();
	}

	/** Links to the manager if not linked and the backoff allows it; whether linked. */
	connect() {
		if (this.#port) return true;
		const now = this.deps.now();
		if (now < this.#retryAt) return false;
		const port = this.deps.connect();
		this.#port = port;
		port.onMessage.addListener((message) => this.deps.onRequest(message));
		port.onDisconnect.addListener(() => this.#lost(port, now));
		for (const report of this.#latest.values()) port.postMessage(report);
		return true;
	}

	/**
	 * @param {Port} port
	 * @param {number} since
	 */
	#lost(port, since) {
		if (this.#port !== port) return;
		this.#port = null;
		const lasted = this.deps.now() - since;
		this.#failures = lasted >= HEALTHY_AFTER_MS ? 1 : this.#failures + 1;
		this.#retryAt = this.deps.now() + reconnectDelay(this.#failures);
	}
}
