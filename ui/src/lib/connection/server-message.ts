import type { ServerMessage } from '#lib/generated/index.js';

/**
 * Every `type` tag of the contract. `satisfies` makes the compiler fail as soon as the
 * generated union gains or loses a variant, so this list cannot drift from the contract.
 */
const SERVER_MESSAGE_TYPES = {
	hello: true,
	devices: true,
	moduleCatalog: true,
	config: true,
	liveState: true,
	error: true,
	pong: true
} as const satisfies Record<ServerMessage['type'], true>;

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isServerMessageType(value: unknown): value is ServerMessage['type'] {
	return typeof value === 'string' && Object.hasOwn(SERVER_MESSAGE_TYPES, value);
}

/**
 * Parses a raw WebSocket frame sent by the manager.
 *
 * Only the envelope (JSON object with a known `type`) is checked: the manager is the
 * source of the contract and both sides are generated from the same Rust types.
 * Returns `undefined` for anything that is not a recognisable server message.
 */
export function parseServerMessage(raw: unknown): ServerMessage | undefined {
	if (typeof raw !== 'string') return undefined;

	let data: unknown;
	try {
		data = JSON.parse(raw);
	} catch {
		return undefined;
	}

	if (!isRecord(data) || !isServerMessageType(data.type)) return undefined;
	return data as ServerMessage;
}
