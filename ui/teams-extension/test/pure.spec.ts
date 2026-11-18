import manifest from '../manifest.json';
import { describe, expect, it, vi } from 'vitest';
import { reconnectDelay } from '../lib/backoff.js';
import { cameraOnFromLabel, mutedFromLabel } from '../lib/labels.js';
import { meetingFromLabel } from '../lib/meetings.js';
import { HEALTHY_AFTER_MS, Relay } from '../lib/relay.js';
import { unreadCountFromTitle } from '../lib/title.js';

describe('unreadCountFromTitle', () => {
	it('reads the count in front of the title', () => {
		expect(unreadCountFromTitle('(3) Chat | Microsoft Teams')).toBe(3);
	});
	it('is zero without a count', () => {
		expect(unreadCountFromTitle('Chat | Microsoft Teams')).toBe(0);
		expect(unreadCountFromTitle('Chat (3)')).toBe(0);
	});
});

describe('labels', () => {
	it.each([
		['Couper le micro', false],
		['Réactiver le micro', true],
		['Mute mic', false],
		['Unmute', true],
		['Autre chose', null],
		[undefined, null]
	])('microphone %s → muted %s', (label, muted) => {
		expect(mutedFromLabel(label)).toBe(muted);
	});
	it.each([
		['Désactiver la caméra', true],
		['Activer la caméra', false],
		['Turn camera off', true],
		['Turn camera on', false],
		['Partager', null]
	])('camera %s → on %s', (label, on) => {
		expect(cameraOnFromLabel(label)).toBe(on);
	});
});

describe('meetingFromLabel', () => {
	const at = (y: number, m: number, d: number, h: number, min: number) =>
		Math.floor(new Date(y, m, d, h, min).getTime() / 1000);

	it('reads a French slot', () => {
		expect(
			meetingFromLabel('Point d’équipe, 10:00 à 11:30, lundi 17 novembre 2026, Alice')
		).toEqual({
			subject: 'Point d’équipe',
			startsAt: at(2026, 10, 17, 10, 0),
			endsAt: at(2026, 10, 17, 11, 30)
		});
	});
	it('reads an English slot with 12-hour times', () => {
		expect(
			meetingFromLabel('Standup, 11:30 PM to 12:15 AM, Monday, November 17, 2026, Bob')
		).toEqual({
			subject: 'Standup',
			startsAt: at(2026, 10, 17, 23, 30),
			endsAt: at(2026, 10, 18, 0, 15)
		});
	});
	it('refuses what is not a meeting', () => {
		expect(meetingFromLabel('Occupé')).toBeNull();
		expect(meetingFromLabel(', 10:00 à 11:00, lundi 17 novembre 2026')).toBeNull();
		expect(meetingFromLabel('Sujet, 10:00 à 11:00, un jour inconnu')).toBeNull();
	});
});

describe('reconnectDelay', () => {
	it('grows from one second to a minute', () => {
		expect([0, 1, 2, 3, 20].map(reconnectDelay)).toEqual([0, 1000, 2000, 4000, 60_000]);
	});
});

function fakePort() {
	const disconnect: (() => void)[] = [];
	const message: ((m: unknown) => void)[] = [];
	return {
		sent: [] as unknown[],
		postMessage(m: unknown) {
			this.sent.push(m);
		},
		onMessage: { addListener: (l: (m: unknown) => void) => message.push(l) },
		onDisconnect: { addListener: (l: () => void) => disconnect.push(l) },
		drop: () => disconnect.forEach((l) => l()),
		receive: (m: unknown) => message.forEach((l) => l(m))
	};
}

describe('Relay', () => {
	function relayWith() {
		let time = 0;
		const ports: ReturnType<typeof fakePort>[] = [];
		const onRequest = vi.fn();
		const relay = new Relay({
			connect: () => {
				ports.push(fakePort());
				return ports.at(-1)!;
			},
			now: () => time,
			onRequest
		});
		return { relay, ports, onRequest, advance: (ms: number) => (time += ms) };
	}

	it('sends a report once, even when it opens the connection', () => {
		const { relay, ports } = relayWith();
		relay.report({ type: 'unread', count: 2 } as never);
		relay.report({ type: 'unread', count: 3 } as never);
		expect(ports[0].sent).toEqual([
			{ type: 'unread', count: 2 },
			{ type: 'unread', count: 3 }
		]);
	});

	it('hands manager requests over', () => {
		const { relay, ports, onRequest } = relayWith();
		relay.connect();
		ports[0].receive({ type: 'do', action: 'hangUp' });
		expect(onRequest).toHaveBeenCalledWith({ type: 'do', action: 'hangUp' });
	});

	it('replays the latest reports on a new connection, after the backoff', () => {
		const { relay, ports, advance } = relayWith();
		relay.report({ type: 'unread', count: 1 } as never);
		ports[0].drop();
		relay.report({ type: 'unread', count: 4 } as never);
		expect(ports).toHaveLength(1);
		expect(relay.connect()).toBe(false);
		advance(1000);
		expect(relay.connect()).toBe(true);
		expect(ports[1].sent).toEqual([{ type: 'unread', count: 4 }]);
	});

	it('backs off further while connections keep failing, and resets after a healthy one', () => {
		const { relay, ports, advance } = relayWith();
		relay.connect();
		ports[0].drop();
		advance(1000);
		relay.connect();
		ports[1].drop();
		advance(1999);
		expect(relay.connect()).toBe(false);
		advance(1);
		expect(relay.connect()).toBe(true);
		advance(HEALTHY_AFTER_MS);
		ports[2].drop();
		advance(1000);
		expect(relay.connect()).toBe(true);
	});

	it('ignores the end of a connection already replaced', () => {
		const { relay, ports, advance } = relayWith();
		relay.connect();
		ports[0].drop();
		advance(1000);
		relay.connect();
		ports[0].drop();
		expect(relay.connect()).toBe(true);
		expect(ports).toHaveLength(2);
	});
});

describe('the manifest', () => {
	// A change of privileges must show as a diff of this test in review.
	// No `use_dynamic_url`: the modules a content script imports resolve against
	// the static extension URL, which that option makes unreachable (the import
	// fails and nothing is ever reported).
	it('carries a version a release can set (dotted numbers)', () => {
		expect(manifest.version).toMatch(/^\d+(\.\d+){1,3}$/);
	});
	it('asks for nothing beyond what the extension needs', () => {
		const origins = ['https://teams.cloud.microsoft/*', 'https://outlook.office.com/*'];
		expect(manifest.permissions).toEqual(['nativeMessaging', 'alarms']);
		expect(manifest.host_permissions).toEqual(['https://teams.cloud.microsoft/*']);
		expect(manifest.content_scripts.map((script) => script.matches)).toEqual([origins]);
		expect(manifest.web_accessible_resources).toEqual([
			{ resources: ['lib/*.js'], matches: origins }
		]);
		expect(manifest).not.toHaveProperty('externally_connectable');
	});
});
