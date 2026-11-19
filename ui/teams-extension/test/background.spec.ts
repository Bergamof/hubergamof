import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const BACKGROUND = '../background.js';
const VERSION = '9.9.9';
const EXTENSION_ID = 'extension-id';
const TEAMS_ORIGIN = 'https://teams.cloud.microsoft';
const CALENDAR_ORIGIN = 'https://outlook.office.com';

type Listener = (...args: unknown[]) => void;

/** A `chrome` fake keeping what the worker registers, to drive it like the browser would. */
function fakeChrome() {
	const port = {
		postMessage: vi.fn(),
		onMessage: { addListener: vi.fn() },
		onDisconnect: { addListener: vi.fn() }
	};
	let onRuntimeMessage: Listener | undefined;
	const chrome = {
		runtime: {
			id: EXTENSION_ID,
			getManifest: () => ({ version: VERSION }),
			connectNative: vi.fn(() => port),
			onMessage: { addListener: (listener: Listener) => (onRuntimeMessage = listener) },
			onStartup: { addListener: vi.fn() },
			onInstalled: { addListener: vi.fn() }
		},
		tabs: {
			query: vi.fn().mockResolvedValue([{ id: 7 }]),
			sendMessage: vi.fn().mockResolvedValue(undefined)
		},
		alarms: { create: vi.fn(), onAlarm: { addListener: vi.fn() } }
	};
	return {
		chrome,
		port,
		fromManager: () => port.onMessage.addListener.mock.calls[0][0] as Listener,
		fromPage: (message: unknown, sender: unknown) => onRuntimeMessage?.(message, sender)
	};
}

async function start() {
	const fake = fakeChrome();
	vi.stubGlobal('chrome', fake.chrome);
	vi.resetModules();
	// By a variable: the worker uses the `chrome` API, which svelte-check has no types for.
	await import(/* @vite-ignore */ BACKGROUND);
	return fake;
}

const teamsFrame = { id: EXTENSION_ID, origin: TEAMS_ORIGIN, frameId: 0, tab: { id: 3 } };

describe('background worker', () => {
	beforeEach(() => vi.useFakeTimers());
	afterEach(() => {
		vi.useRealTimers();
		vi.unstubAllGlobals();
	});

	it('announces its version to the manager as soon as it is linked', async () => {
		const { port, chrome } = await start();
		expect(chrome.runtime.connectNative).toHaveBeenCalledWith('com.hubergamof.teams');
		expect(port.postMessage).toHaveBeenCalledWith({ type: 'hello', version: VERSION });
	});

	it('relays the report of Teams itself, not the one of another frame or extension', async () => {
		const { port, fromPage } = await start();
		const call = { type: 'call', inCall: true };
		fromPage({ report: call }, { ...teamsFrame, frameId: 2 });
		fromPage({ report: call }, { ...teamsFrame, id: 'another-extension' });
		fromPage({ report: call }, { ...teamsFrame, origin: 'https://example.org' });
		expect(port.postMessage).not.toHaveBeenCalledWith(call);

		fromPage({ report: call }, teamsFrame);
		expect(port.postMessage).toHaveBeenCalledWith(call);
	});

	it('takes the meetings from the calendar only', async () => {
		const { port, fromPage } = await start();
		const meetings = { type: 'meetings', meetings: [] };
		fromPage({ report: meetings }, teamsFrame);
		expect(port.postMessage).not.toHaveBeenCalledWith(meetings);
		fromPage({ report: meetings }, { ...teamsFrame, origin: CALENDAR_ORIGIN, frameId: 1 });
		expect(port.postMessage).toHaveBeenCalledWith(meetings);
	});

	it('passes a known action to the tab of the call, in the Teams frame', async () => {
		const { chrome, fromManager, fromPage } = await start();
		fromPage({ report: { type: 'call' } }, teamsFrame);
		fromManager()({ type: 'do', action: 'hangUp' });
		await vi.waitFor(() => expect(chrome.tabs.sendMessage).toHaveBeenCalled());
		expect(chrome.tabs.sendMessage).toHaveBeenCalledWith(
			3,
			{ type: 'do', action: 'hangUp' },
			{ frameId: 0 }
		);
	});

	it('drops what the manager may not ask, and rebuilds what it passes', async () => {
		const { chrome, fromManager } = await start();
		const ask = fromManager();
		ask({ type: 'do', action: 'formatDisk' });
		ask({ type: 'openConversation', id: '' });
		ask({ type: 'openConversation', id: 'x'.repeat(401) });
		ask({ type: 'openPanel', panel: 'settings' });
		ask({ type: 'unknown' });
		await Promise.resolve();
		expect(chrome.tabs.sendMessage).not.toHaveBeenCalled();

		ask({ type: 'openConversation', id: 'chat:1', extra: 'ignored' });
		await vi.waitFor(() => expect(chrome.tabs.sendMessage).toHaveBeenCalled());
		expect(chrome.tabs.sendMessage).toHaveBeenCalledWith(
			7,
			{ type: 'openConversation', id: 'chat:1' },
			{ frameId: 0 }
		);

		ask({ type: 'openPanel', panel: 'activity', extra: 'ignored' });
		await vi.waitFor(() => expect(chrome.tabs.sendMessage).toHaveBeenCalledTimes(2));
		expect(chrome.tabs.sendMessage).toHaveBeenLastCalledWith(
			7,
			{ type: 'openPanel', panel: 'activity' },
			{ frameId: 0 }
		);
	});
});
