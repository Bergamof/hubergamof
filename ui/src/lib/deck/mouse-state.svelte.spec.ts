import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
	DeviceConfig,
	DeviceInfo,
	MouseConfig,
	MouseInfo,
	RadialMenuConfig
} from '#lib/generated/index.js';
import { DeckState, UPDATE_TIMEOUT_MS } from './deck-state.svelte.js';
import type { DevicePreference } from './device-preference.js';
import { setBrightness } from './config-edits.js';
import { setDpi, setHoldMs } from './mouse-edits.js';
import { FakeConnection } from './fake-connection.js';

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

	it('fails what waits for a mouse that is no longer followed', async () => {
		const { connection, mice } = setup();
		connection.receive({ type: 'mice', mice: [mouse('a'), mouse('b')] });
		connection.receive({ type: 'mouseConfig', serial: 'a', config: mouseConfig('a', 0) });
		const inFlight = mice.edit(setDpi(800), { notify: false });
		const queued = mice.edit(setDpi(900), { notify: false });
		mice.select('b');
		await expect(inFlight).resolves.toMatchObject({ saved: false });
		await expect(queued).resolves.toMatchObject({ saved: false });
		expect(mice.config).toBeUndefined();
	});

	it('fails what waits for a mouse that went away, and takes it back when it returns', async () => {
		const { connection, mice } = setup();
		connection.receive({ type: 'mice', mice: [mouse('a')] });
		connection.receive({ type: 'mouseConfig', serial: 'a', config: mouseConfig('a', 0) });
		const outcome = mice.edit(setDpi(800), { notify: false });
		connection.receive({ type: 'mice', mice: [] });
		await expect(outcome).resolves.toMatchObject({ saved: false });
		connection.receive({ type: 'mice', mice: [mouse('a')] });
		expect(connection.sent.at(-1)).toEqual({ type: 'selectMouse', serial: 'a' });
		connection.receive({ type: 'mouseConfig', serial: 'a', config: mouseConfig('a', 3) });
		void mice.edit(setDpi(700));
		expect(connection.sent.at(-1)).toMatchObject({ type: 'updateMouseConfig' });
	});

	it('ignores the configuration a mouse sends after another one was chosen', () => {
		const { connection, mice } = setup();
		connection.receive({ type: 'mice', mice: [mouse('a'), mouse('b')] });
		mice.select('b');
		connection.receive({ type: 'mouseConfig', serial: 'a', config: mouseConfig('a', 9) });
		expect(mice.config).toBeUndefined();
	});

	it('keeps the radial menu edit when the mouse goes away', async () => {
		const { connection, mice } = setup();
		connection.receive({ type: 'radialMenu', config: radialMenu(0) });
		connection.receive({ type: 'mice', mice: [mouse('a')] });
		const outcome = mice.editRadialMenu(setHoldMs(300));
		connection.receive({ type: 'mice', mice: [] });
		connection.receive({ type: 'radialMenu', config: radialMenu(1, 300) });
		await expect(outcome).resolves.toEqual({ saved: true });
	});

	it('shows one notice when an error answers the deck and the mice at once', async () => {
		const { connection, mice, state } = setup();
		connection.receive({ type: 'devices', devices: [device('d')] });
		connection.receive({ type: 'config', serial: 'd', config: deckConfig('d', 0) });
		connection.receive({ type: 'mice', mice: [mouse('a')] });
		connection.receive({ type: 'mouseConfig', serial: 'a', config: mouseConfig('a', 0) });
		const deckOutcome = state.edit(setBrightness(10), { notify: false });
		const mouseOutcome = mice.edit(setDpi(800), { notify: false });
		connection.receive({ type: 'error', code: 'internal', message: 'boom' });
		await expect(deckOutcome).resolves.toMatchObject({ saved: false });
		await expect(mouseOutcome).resolves.toMatchObject({ saved: false });
		expect(state.notices).toHaveLength(1);
	});

	it('fails the edits of the menu and of the mouse when the connection starts over', async () => {
		const { connection, mice } = setup();
		connection.receive({ type: 'radialMenu', config: radialMenu(0) });
		const outcome = mice.editRadialMenu(setHoldMs(300), { notify: false });
		connection.receive({ type: 'hello', version: '1' });
		await expect(outcome).resolves.toMatchObject({ saved: false });
		expect(mice.pending).toBe(false);
	});
});

function device(serial: string): DeviceInfo {
	return {
		serial,
		model: 'plus',
		displayName: `Stream Deck+ ${serial}`,
		capabilities: {
			keyColumns: 4,
			keyRows: 2,
			keyImageSize: { width: 120, height: 120 },
			dialCount: 4,
			touchStrip: { width: 800, height: 100 }
		}
	};
}

function deckConfig(serial: string, revision: number): DeviceConfig {
	return {
		schemaVersion: 3,
		serial,
		model: 'plus',
		brightness: 50,
		lockSleepSeconds: 5,
		home: [],
		modules: [],
		dials: [{ kind: 'none' }, { kind: 'none' }, { kind: 'none' }, { kind: 'none' }],
		strip: {
			showSeconds: true,
			showPomodoroProgress: true,
			systemRefreshSeconds: 2,
			showHistory: true,
			loadAlertPercent: 85
		},
		theme: 'light',
		revision
	};
}
