import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
	ClientMessage,
	DeviceConfig,
	DeviceInfo,
	ServerMessage
} from '#lib/generated/index.js';
import { addKey, removeKey, setBrightness } from './config-edits.js';
import { DeckState, UPDATE_TIMEOUT_MS, type DeckConnection } from './deck-state.svelte.js';
import type { DevicePreference } from './device-preference.js';

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

	lastSent(): ClientMessage | undefined {
		return this.sent.at(-1);
	}
}

class MemoryPreference implements DevicePreference {
	constructor(public serial: string | null = null) {}

	load(): string | null {
		return this.serial;
	}

	save(serial: string): void {
		this.serial = serial;
	}
}

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

function config(serial: string, revision: number, brightness = 50): DeviceConfig {
	return {
		schemaVersion: 1,
		serial,
		model: 'plus',
		brightness,
		home: [],
		modules: [],
		dials: [null, null, null, null],
		revision
	};
}

function setup(preference = new MemoryPreference()) {
	const connection = new FakeConnection();
	const state = new DeckState(connection, preference);
	const stop = state.listen();
	return { connection, state, preference, stop };
}

/** A state following device `A` at revision 1. */
function following() {
	const context = setup();
	context.connection.receive({ type: 'devices', devices: [device('A')] });
	context.connection.receive({ type: 'config', serial: 'A', config: config('A', 1) });
	context.connection.sent.length = 0;
	return context;
}

describe('device selection', () => {
	it('follows the only device and selects it again on every list', () => {
		const { connection, state } = setup();
		connection.receive({ type: 'devices', devices: [device('A')] });
		expect(state.selectedSerial).toBe('A');
		expect(state.device?.serial).toBe('A');
		connection.receive({ type: 'devices', devices: [device('A')] });
		expect(connection.sent).toEqual([
			{ type: 'selectDevice', serial: 'A', liveState: false },
			{ type: 'selectDevice', serial: 'A', liveState: false }
		]);
	});

	it('prefers the remembered device, and remembers the chosen one', () => {
		const { connection, state, preference } = setup(new MemoryPreference('B'));
		connection.receive({ type: 'devices', devices: [device('A'), device('B')] });
		expect(state.selectedSerial).toBe('B');
		state.select('A');
		expect(preference.serial).toBe('A');
		expect(connection.lastSent()).toEqual({ type: 'selectDevice', serial: 'A', liveState: false });
	});

	it('keeps the followed device when the remembered one comes back, and selects it again', () => {
		const { connection, state } = setup(new MemoryPreference('B'));
		connection.receive({ type: 'devices', devices: [device('A')] });
		connection.receive({ type: 'devices', devices: [device('A'), device('B')] });
		expect(state.selectedSerial).toBe('A');

		connection.receive({ type: 'devices', devices: [device('B')] });
		expect(state.selectedSerial).toBe('B');
		connection.receive({ type: 'devices', devices: [device('A'), device('B')] });
		expect(state.selectedSerial).toBe('B');
	});

	it('ignores the late configuration of the previous device after a switch', () => {
		const { connection, state } = following();
		connection.receive({ type: 'devices', devices: [device('A'), device('B')] });
		state.select('B');
		connection.receive({ type: 'config', serial: 'A', config: config('A', 2) });
		expect(state.config).toBeUndefined();
		connection.receive({ type: 'config', serial: 'B', config: config('B', 4) });
		expect(state.config?.serial).toBe('B');
	});

	it('forgets the configuration of a device that went away', () => {
		const { connection, state } = following();
		connection.receive({ type: 'devices', devices: [] });
		expect(state.selectedSerial).toBeUndefined();
		expect(state.config).toBeUndefined();
		expect(connection.sent).toEqual([]);
	});

	it('ignores the configuration of another device', () => {
		const { connection, state } = following();
		connection.receive({ type: 'config', serial: 'B', config: config('B', 9) });
		expect(state.config?.serial).toBe('A');
	});

	it('stores the catalog and stops listening when asked', () => {
		const { connection, state, stop } = setup();
		const clock = {
			kind: 'clock',
			name: 'Horloge',
			description: '',
			capabilities: { key: false, dial: true, touchZone: true },
			settings: [],
			icon: null,
			settingIcons: {},
			defaultSettings: {}
		};
		connection.receive({ type: 'moduleCatalog', modules: [clock] });
		expect(state.moduleOf('clock')).toEqual(clock);
		expect(state.moduleOf('intellij')).toBeUndefined();
		stop();
		connection.receive({ type: 'devices', devices: [device('A')] });
		expect(state.devices).toEqual([]);
	});
});

describe('configuration updates', () => {
	afterEach(() => {
		vi.useRealTimers();
	});

	it('send the edited configuration with the saved revision and show it at once', async () => {
		const { connection, state } = following();
		const outcome = state.edit(setBrightness(80));
		expect(connection.sent).toEqual([
			{ type: 'updateConfig', serial: 'A', config: config('A', 1, 80) }
		]);
		expect(state.config?.brightness).toBe(80);
		connection.receive({ type: 'config', serial: 'A', config: config('A', 2, 80) });
		await expect(outcome).resolves.toEqual({ saved: true });
	});

	it('wait for the answer, then send the queued edits on the new revision in one update', async () => {
		const { connection, state } = following();
		const first = state.edit(setBrightness(60));
		const queued = [state.edit(setBrightness(70)), state.edit(setBrightness(80))];
		expect(connection.sent).toHaveLength(1);
		expect(state.config?.brightness).toBe(80);

		connection.receive({ type: 'config', serial: 'A', config: config('A', 2, 60) });
		await expect(first).resolves.toEqual({ saved: true });
		expect(connection.sent).toHaveLength(2);
		expect(connection.lastSent()).toEqual({
			type: 'updateConfig',
			serial: 'A',
			config: config('A', 2, 80)
		});
		connection.receive({ type: 'config', serial: 'A', config: config('A', 3, 80) });
		await expect(Promise.all(queued)).resolves.toEqual([{ saved: true }, { saved: true }]);
		expect(connection.sent).toHaveLength(2);
		expect(state.config).toEqual(config('A', 3, 80));
	});

	it('apply queued key edits on the configuration they were made on', () => {
		const { connection, state } = following();
		const clock = {
			kind: 'clock',
			name: 'Horloge',
			description: '',
			capabilities: { key: true, dial: true, touchZone: true },
			settings: [],
			icon: null,
			settingIcons: {},
			defaultSettings: {}
		};
		state.edit(addKey(clock));
		state.edit(addKey(clock));
		state.edit(removeKey(0));
		expect(state.config?.home).toEqual(['clock-2']);
		const sentFirst = connection.lastSent();
		if (sentFirst?.type !== 'updateConfig') throw new Error('expected an update');
		connection.receive({
			type: 'config',
			serial: 'A',
			config: { ...sentFirst.config, revision: 2 }
		});
		expect(connection.lastSent()).toMatchObject({ config: { home: ['clock-2'], revision: 2 } });
	});

	it.each(['invalidConfig', 'internal', 'unknownDevice', 'invalidMessage'] as const)(
		'drop every pending edit on a %s error, and explain it once',
		async (code) => {
			const { connection, state } = following();
			const outcomes = [state.edit(setBrightness(60)), state.edit(setBrightness(70))];
			connection.receive({ type: 'error', code, message: 'details' });
			expect(state.config?.brightness).toBe(50);
			expect(state.notices).toHaveLength(1);
			const [first] = await Promise.all(outcomes);
			expect(first).toMatchObject({ saved: false, problem: { text: state.notices[0].text } });

			state.edit(setBrightness(90));
			expect(connection.sent).toHaveLength(2);
		}
	);

	it('keep the French text apart from the English details of the manager', () => {
		const { connection, state } = following();
		state.edit(setBrightness(60));
		connection.receive({ type: 'error', code: 'invalidConfig', message: 'not an absolute path' });
		expect(state.notices[0]).toMatchObject({
			text: 'Le gestionnaire a refusé la modification : vérifiez les réglages.',
			detail: 'not an absolute path'
		});
	});

	it('keep waiting when another client saves meanwhile, then drop on the stale refusal', async () => {
		const { connection, state } = following();
		const mine = state.edit(setBrightness(60));
		state.edit(setBrightness(70));
		connection.receive({ type: 'config', serial: 'A', config: config('A', 2, 20) });
		expect(connection.sent).toHaveLength(1);
		expect(state.config?.brightness).toBe(70);

		connection.receive({ type: 'error', code: 'staleRevision', message: 'stale' });
		await expect(mine).resolves.toMatchObject({ saved: false });
		expect(state.config).toEqual(config('A', 2, 20));
		expect(state.notices[0].text).toMatch(/modifiée ailleurs/);
	});

	it('give up an update the manager never answers', async () => {
		vi.useFakeTimers();
		const { connection, state } = following();
		const outcome = state.edit(setBrightness(60));
		vi.advanceTimersByTime(UPDATE_TIMEOUT_MS);
		await expect(outcome).resolves.toMatchObject({ saved: false });
		expect(state.config?.brightness).toBe(50);
		expect(state.notices[0].text).toMatch(/pas répondu/);
		state.edit(setBrightness(70));
		expect(connection.sent).toHaveLength(2);
	});

	it('keep a reported warning until dismissed', () => {
		const { state } = following();
		state.report({ text: 'Réglage perdu.', detail: null });
		state.report({ text: 'Autre réglage perdu.', detail: null });
		expect(state.notices.map((notice) => notice.text)).toEqual([
			'Réglage perdu.',
			'Autre réglage perdu.'
		]);
		state.dismiss(state.notices[0].id);
		expect(state.notices.map((notice) => notice.text)).toEqual(['Autre réglage perdu.']);
	});

	it('never give up an update once saved', async () => {
		vi.useFakeTimers();
		const { connection, state } = following();
		const first = state.edit(setBrightness(60));
		const second = state.edit(setBrightness(70));
		connection.receive({ type: 'config', serial: 'A', config: config('A', 2, 60) });
		vi.advanceTimersByTime(UPDATE_TIMEOUT_MS - 1);
		connection.receive({ type: 'config', serial: 'A', config: config('A', 3, 70) });
		vi.advanceTimersByTime(UPDATE_TIMEOUT_MS);
		await expect(Promise.all([first, second])).resolves.toEqual([{ saved: true }, { saved: true }]);
		expect(state.notices).toEqual([]);
	});

	it('let the caller show the failure itself', async () => {
		const { connection, state } = following();
		const outcome = state.edit(setBrightness(60), { notify: false });
		connection.receive({ type: 'error', code: 'invalidConfig', message: 'bad' });
		await expect(outcome).resolves.toMatchObject({ saved: false });
		expect(state.notices).toEqual([]);
	});

	it('start again after a reconnection', async () => {
		const { connection, state } = following();
		const lost = state.edit(setBrightness(60));
		connection.receive({ type: 'hello', version: '1.0.0' });
		await expect(lost).resolves.toMatchObject({ saved: false });
		expect(state.config?.brightness).toBe(50);
		expect(state.notices[0].text).toMatch(/connexion/);

		connection.receive({ type: 'devices', devices: [device('A')] });
		expect(connection.lastSent()).toEqual({ type: 'selectDevice', serial: 'A', liveState: false });
		connection.receive({ type: 'config', serial: 'A', config: config('A', 5, 30) });
		state.edit(setBrightness(40));
		expect(connection.lastSent()).toEqual({
			type: 'updateConfig',
			serial: 'A',
			config: config('A', 5, 40)
		});
	});

	it('fail at once when the connection is down', async () => {
		const { connection, state } = following();
		connection.open = false;
		await expect(state.edit(setBrightness(60))).resolves.toMatchObject({ saved: false });
		expect(state.config?.brightness).toBe(50);
		expect(state.notices).toHaveLength(1);
	});

	it('fail before any configuration is known', async () => {
		const { connection, state } = setup();
		await expect(state.edit(setBrightness(60))).resolves.toMatchObject({ saved: false });
		expect(connection.sent).toEqual([]);
	});
});

describe('notices', () => {
	it('report errors outside updates and can be dismissed', () => {
		const { connection, state } = following();
		connection.receive({ type: 'error', code: 'unknownDevice', message: 'gone' });
		connection.receive({ type: 'error', code: 'internal', message: 'boom' });
		expect(state.notices).toMatchObject([
			{ text: 'Cet appareil n’est plus connecté.', detail: null },
			{ text: 'Le gestionnaire a signalé une erreur.', detail: 'boom' }
		]);
		state.dismiss(state.notices[0].id);
		expect(state.notices.map((notice) => notice.detail)).toEqual(['boom']);
	});
});
