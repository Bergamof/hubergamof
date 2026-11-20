import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
	ClientMessage,
	MouseConfig,
	MouseInfo,
	RadialMenuConfig,
	ServerMessage
} from '#lib/generated/index.js';
import { DeckState, UPDATE_TIMEOUT_MS, type DeckConnection } from './deck-state.svelte.js';
import type { DevicePreference } from './device-preference.js';
import { setDpi, setHoldMs } from './mouse-edits.js';

class FakeConnection implements DeckConnection {
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
}

const NO_PREFERENCE: DevicePreference = { load: () => null, save: () => undefined };

function mouse(serial: string): MouseInfo {
	return {
		serial,
		model: 'mxMaster4',
		displayName: `MX Master 4 ${serial}`,
		capabilities: { controls: [], thumbWheel: true, haptics: true }
	};
}

function mouseConfig(serial: string, revision: number, dpi = 1600): MouseConfig {
	return {
		schemaVersion: 1,
		serial,
		model: 'mxMaster4',
		bindings: {},
		pointer: { dpi },
		wheel: { mode: 'ratchet', autoThreshold: 20, natural: false, hiRes: true },
		thumbWheel: { speed: 5, invert: false },
		haptics: { level: 'medium', events: {} },
		revision
	};
}

function radialMenu(revision: number, holdMs = 280): RadialMenuConfig {
	return { schemaVersion: 1, holdMs, animate: true, haptics: true, entries: [], revision };
}

function setup() {
	const connection = new FakeConnection();
	const state = new DeckState(connection, NO_PREFERENCE);
	state.listen();
	return { connection, state, mice: state.mice };
}

afterEach(() => {
	vi.useRealTimers();
});

describe('MouseState', () => {
	it('follows the first mouse and selects it again on every list', () => {
		const { connection, mice } = setup();
		connection.receive({ type: 'mice', mice: [mouse('a'), mouse('b')] });
		expect(mice.selectedSerial).toBe('a');
		expect(mice.mouse?.displayName).toBe('MX Master 4 a');
		connection.receive({ type: 'mice', mice: [mouse('a'), mouse('b')] });
		expect(connection.sent.filter((m) => m.type === 'selectMouse')).toHaveLength(2);
	});

	it('keeps the mouse chosen, and follows another one when it goes away', () => {
		const { connection, mice } = setup();
		connection.receive({ type: 'mice', mice: [mouse('a'), mouse('b')] });
		mice.select('b');
		mice.select('b');
		expect(mice.selectedSerial).toBe('b');
		connection.receive({ type: 'mice', mice: [mouse('a'), mouse('b')] });
		expect(mice.selectedSerial).toBe('b');
		connection.receive({ type: 'mice', mice: [mouse('a')] });
		expect(mice.selectedSerial).toBe('a');
		connection.receive({ type: 'mice', mice: [] });
		expect(mice.selectedSerial).toBeUndefined();
		expect(mice.mouse).toBeUndefined();
	});

	it('shows the configuration of the selected mouse only', () => {
		const { connection, mice } = setup();
		connection.receive({ type: 'mice', mice: [mouse('a')] });
		connection.receive({ type: 'mouseConfig', serial: 'other', config: mouseConfig('other', 0) });
		expect(mice.config).toBeUndefined();
		connection.receive({ type: 'mouseConfig', serial: 'a', config: mouseConfig('a', 0) });
		expect(mice.config?.serial).toBe('a');
	});

	it('shows an edit at once and settles it when the manager saves it', async () => {
		const { connection, mice } = setup();
		connection.receive({ type: 'mice', mice: [mouse('a')] });
		connection.receive({ type: 'mouseConfig', serial: 'a', config: mouseConfig('a', 4) });
		const outcome = mice.edit(setDpi(800));
		expect(mice.config?.pointer.dpi).toBe(800);
		expect(connection.sent.at(-1)).toMatchObject({ type: 'updateMouseConfig', serial: 'a' });
		connection.receive({ type: 'mouseConfig', serial: 'a', config: mouseConfig('a', 5, 800) });
		await expect(outcome).resolves.toEqual({ saved: true });
	});

	it('edits the radial menu the same way', async () => {
		const { connection, mice } = setup();
		const early = await mice.editRadialMenu(setHoldMs(300), { notify: false });
		expect(early.saved).toBe(false);
		connection.receive({ type: 'radialMenu', config: radialMenu(2) });
		const outcome = mice.editRadialMenu(setHoldMs(300));
		expect(mice.radialMenu?.holdMs).toBe(300);
		expect(connection.sent.at(-1)).toMatchObject({ type: 'updateRadialMenu' });
		connection.receive({ type: 'radialMenu', config: radialMenu(3, 300) });
		await expect(outcome).resolves.toEqual({ saved: true });
	});

	it('refuses an edit when no mouse is selected, with a notice', async () => {
		const { mice, state } = setup();
		const outcome = await mice.edit(setDpi(800));
		expect(outcome.saved).toBe(false);
		expect(state.notices).toHaveLength(1);
	});

	it('gives an error to the update in flight instead of showing it twice', async () => {
		const { connection, mice, state } = setup();
		connection.receive({ type: 'mice', mice: [mouse('a')] });
		connection.receive({ type: 'mouseConfig', serial: 'a', config: mouseConfig('a', 0) });
		const outcome = mice.edit(setDpi(7));
		expect(mice.pending).toBe(true);
		connection.receive({ type: 'error', code: 'invalidConfig', message: 'dpi' });
		await expect(outcome).resolves.toMatchObject({ saved: false });
		expect(mice.pending).toBe(false);
		expect(state.notices).toHaveLength(1);
	});

	it('shows an error nobody waits for', () => {
		const { connection, state } = setup();
		connection.receive({ type: 'error', code: 'invalidConfig', message: 'dpi' });
		expect(state.notices).toHaveLength(1);
	});

	it('gives up an update the manager does not answer', async () => {
		vi.useFakeTimers();
		const { connection, mice } = setup();
		connection.receive({ type: 'mice', mice: [mouse('a')] });
		connection.receive({ type: 'mouseConfig', serial: 'a', config: mouseConfig('a', 0) });
		const outcome = mice.edit(setDpi(800), { notify: false });
		vi.advanceTimersByTime(UPDATE_TIMEOUT_MS);
		await expect(outcome).resolves.toMatchObject({ saved: false });
	});

	it('drops what waits when the connection starts over', async () => {
		const { connection, mice } = setup();
		connection.receive({ type: 'mice', mice: [mouse('a')] });
		connection.receive({ type: 'mouseConfig', serial: 'a', config: mouseConfig('a', 0) });
		const outcome = mice.edit(setDpi(800), { notify: false });
		connection.receive({ type: 'hello', version: '1' });
		await expect(outcome).resolves.toMatchObject({ saved: false });
	});
});
