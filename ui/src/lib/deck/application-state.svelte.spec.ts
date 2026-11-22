import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ApplicationModuleView } from '#lib/generated/index.js';
import { DeckState } from './deck-state.svelte.js';
import { UPDATE_TIMEOUT_MS } from './edit-queue.svelte.js';
import { FakeConnection } from './fake-connection.js';
import type { DevicePreference } from './device-preference.js';

const NO_PREFERENCE: DevicePreference = { load: () => null, save: () => undefined };

function pomodoro(settings: Record<string, number> = { workMinutes: 25 }): ApplicationModuleView {
	return { kind: 'pomodoro', status: 'Focus 1/4\n25:00 · prêt', settings };
}

function setup() {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, NO_PREFERENCE);
	deck.listen();
	return { connection, deck, application: deck.application };
}

afterEach(() => {
	vi.useRealTimers();
});

describe('ApplicationState', () => {
	it('follows the modules of the application and whether the computer is locked', () => {
		const { connection, application } = setup();
		expect(application.modules).toEqual([]);
		expect(application.locked).toBe(false);

		connection.receive({ type: 'applicationModules', modules: [pomodoro()] });
		expect(application.viewOf('pomodoro')?.status).toBe('Focus 1/4\n25:00 · prêt');
		expect(application.viewOf('audio')).toBeUndefined();

		connection.receive({ type: 'locked', locked: true });
		expect(application.locked).toBe(true);
		connection.receive({ type: 'locked', locked: false });
		expect(application.locked).toBe(false);
	});

	it('asks the manager to run an action of a module', () => {
		const { connection, application } = setup();
		expect(application.run('pomodoro', 'toggle')).toBe(true);
		expect(connection.lastSent()).toEqual({
			type: 'moduleAction',
			module: 'pomodoro',
			action: 'toggle'
		});
		connection.open = false;
		expect(application.run('pomodoro', 'toggle')).toBe(false);
	});

	it('saves settings and is done once the manager shows them as the module’s', async () => {
		const { connection, application } = setup();
		const saved = application.saveSettings('pomodoro', { workMinutes: 30 });
		expect(connection.lastSent()).toEqual({
			type: 'updateModuleSettings',
			module: 'pomodoro',
			settings: { workMinutes: 30 }
		});
		expect(application.pending).toBe(true);

		// Another change of the module (a tick) is not the answer.
		connection.receive({ type: 'applicationModules', modules: [pomodoro()] });
		expect(application.pending).toBe(true);

		connection.receive({ type: 'applicationModules', modules: [pomodoro({ workMinutes: 30 })] });
		expect(await saved).toEqual({ saved: true });
		expect(application.pending).toBe(false);
	});

	it('reports the refusal of the manager as the outcome of the settings', async () => {
		const { connection, deck } = setup();
		const saved = deck.application.saveSettings('pomodoro', { workMinutes: 0 });
		connection.receive({ type: 'error', code: 'invalidConfig', message: 'workMinutes below 1' });
		const outcome = await saved;
		expect(outcome.saved).toBe(false);
		expect(deck.notices).toEqual([]);
		expect(deck.application.pending).toBe(false);
	});

	it('gives up when the manager does not answer, and when the connection is lost', async () => {
		vi.useFakeTimers();
		const { connection, application } = setup();
		const silent = application.saveSettings('pomodoro', { workMinutes: 30 });
		await vi.advanceTimersByTimeAsync(UPDATE_TIMEOUT_MS);
		expect((await silent).saved).toBe(false);

		const lost = application.saveSettings('pomodoro', { workMinutes: 31 });
		connection.receive({ type: 'hello', version: '1' });
		expect((await lost).saved).toBe(false);
		expect(application.pending).toBe(false);
	});

	it('does not wait when the message cannot be sent, and a newer save supersedes an older', async () => {
		const { connection, application } = setup();
		const first = application.saveSettings('pomodoro', { workMinutes: 30 });
		const second = application.saveSettings('pomodoro', { workMinutes: 31 });
		expect((await first).saved).toBe(false);
		connection.receive({ type: 'applicationModules', modules: [pomodoro({ workMinutes: 31 })] });
		expect(await second).toEqual({ saved: true });

		connection.open = false;
		expect((await application.saveSettings('pomodoro', { workMinutes: 1 })).saved).toBe(false);
	});
});
