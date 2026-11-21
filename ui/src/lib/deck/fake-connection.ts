import type { ClientMessage, ServerMessage } from '#lib/generated/index.js';
import type { DeckConnection } from './deck-state.svelte.js';

/** A connection to the manager that records what is sent and delivers what the test says. */
export class FakeConnection implements DeckConnection {
	readonly sent: ClientMessage[] = [];
	open = true;
	#listener: ((message: ServerMessage) => void) | undefined;

	send(message: ClientMessage): boolean {
		if (!this.open) return false;
		this.sent.push(message);
		return true;
	}

	onMessage(listener: (message: ServerMessage) => void): () => void {
		this.#listener = listener;
		return () => {
			this.#listener = undefined;
		};
	}

	receive(message: ServerMessage): void {
		this.#listener?.(message);
	}

	lastSent(): ClientMessage | undefined {
		return this.sent.at(-1);
	}
}
