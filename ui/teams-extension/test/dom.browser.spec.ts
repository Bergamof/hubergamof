import { afterEach, describe, expect, it, vi } from 'vitest';
import { start } from '../lib/content-main.js';
import {
	frameRole,
	performAction,
	readCall,
	readMeetings,
	readReports,
	readUnreadConversations
} from '../lib/teams-dom.js';

function page(html: string, title = 'Teams'): Document {
	const doc = document.implementation.createHTMLDocument(title);
	doc.body.innerHTML = html;
	return doc;
}

const CONVERSATIONS = `
	<div role="treeitem" aria-level="2" aria-labelledby="a"><span id="a">Alice</span><i data-tid="unread"></i></div>
	<div role="treeitem" aria-level="2" aria-labelledby="b"><span id="b">Bob</span></div>
	<div role="treeitem" aria-level="2" aria-labelledby="c"><span id="c">Carol</span><i data-tid="unread"></i></div>`;

describe('readCall', () => {
	it('is idle without call controls', () => {
		expect(readCall(page('<p>Chat</p>')).phase).toBe('idle');
	});
	it('reads an ongoing call from its button labels', () => {
		const call = readCall(
			page(`<button data-tid="hangup-main-btn"></button>
				<button id="microphone-button" aria-label="Réactiver le micro"></button>
				<button id="video-button" aria-label="Désactiver la caméra"></button>`)
		);
		expect(call).toMatchObject({ phase: 'inCall', muted: true, cameraOn: true });
	});
	it('leaves unknown labels unknown', () => {
		const call = readCall(page('<span data-tid="call-duration">00:10</span>'));
		expect(call).toMatchObject({ phase: 'inCall', muted: null, cameraOn: null });
	});
	it('sees the pre-join screen', () => {
		expect(readCall(page('<div data-tid="calling-prejoin-screen"></div>')).phase).toBe('prejoin');
	});
});

describe('readUnreadConversations', () => {
	it('names the unread conversations only', () => {
		expect(readUnreadConversations(page(CONVERSATIONS))).toEqual(['Alice', 'Carol']);
	});
	it('learns nothing when the list is not on screen', () => {
		expect(readUnreadConversations(page('<p>Calendrier</p>'))).toBeNull();
	});
	it('skips an entry without a name', () => {
		const doc = page('<div role="treeitem" aria-level="2"><i data-tid="unread"></i></div>');
		expect(readUnreadConversations(doc)).toEqual([]);
	});
});

describe('readMeetings', () => {
	const slots = `
		<div role="button" aria-label="Passée, 08:00 à 09:00, lundi 17 novembre 2026, A"></div>
		<div role="button" aria-label="Plus tard, 16:00 à 17:00, lundi 17 novembre 2026, A"></div>
		<div role="button" aria-label="Bientôt, 11:00 à 12:00, lundi 17 novembre 2026, A"></div>
		<div role="button" aria-label="Occupé"></div>`;
	const now = Math.floor(new Date(2026, 10, 17, 10, 0).getTime() / 1000);

	it('keeps upcoming meetings, soonest first', () => {
		expect(readMeetings(page(slots), now)?.map((m) => m.subject)).toEqual(['Bientôt', 'Plus tard']);
	});
	it('learns nothing without any meeting', () => {
		expect(readMeetings(page('<p>rien</p>'), now)).toBeNull();
	});
});

describe('readReports', () => {
	it('reports the count, the names and the call of Teams', () => {
		const doc = page(CONVERSATIONS, '(2) Chat');
		expect(readReports(doc, 'teams.cloud.microsoft', 0).map((r) => r.type)).toEqual([
			'unread',
			'conversations',
			'call'
		]);
	});
	it('says nobody is unread without reading the list when the count is zero', () => {
		const reports = readReports(page(CONVERSATIONS, 'Chat'), 'teams.cloud.microsoft', 0);
		expect(reports[1]).toEqual({ type: 'conversations', unread: [] });
	});
	it('keeps the previous names when the list is hidden', () => {
		const reports = readReports(page('<p/>', '(2) Calendrier'), 'teams.cloud.microsoft', 0);
		expect(reports.map((r) => r.type)).toEqual(['unread', 'call']);
	});
	it('reads only meetings in the calendar frame', () => {
		const doc = page(
			'<div role="button" aria-label="X, 10:00 à 11:00, lundi 17 novembre 2026"></div>'
		);
		expect(readReports(doc, 'outlook.office.com', 0).map((r) => r.type)).toEqual(['meetings']);
	});
	it('reads nothing elsewhere', () => {
		expect(readReports(page('<p/>'), 'example.org', 0)).toEqual([]);
		expect(frameRole('example.org')).toBe('other');
	});
});

describe('performAction', () => {
	it('clicks the control of the action', () => {
		const doc = page('<button id="microphone-button"></button>');
		const click = vi.fn();
		doc.querySelector('button')!.addEventListener('click', click);
		expect(performAction(doc, 'toggleMute')).toBe(true);
		expect(click).toHaveBeenCalledOnce();
	});
	it('falls back to the pre-join switch', () => {
		const doc = page('<input type="checkbox" data-tid="toggle-video">');
		expect(performAction(doc, 'toggleCamera')).toBe(true);
	});
	it('clicks hang up', () => {
		const doc = page('<button data-tid="hangup-main-btn"></button>');
		expect(performAction(doc, 'hangUp')).toBe(true);
	});
	it('does nothing for a missing control or an unsupported action', () => {
		const doc = page('<p/>');
		expect(performAction(doc, 'hangUp')).toBe(false);
		expect(performAction(doc, 'accept')).toBe(false);
	});
});

describe('start', () => {
	afterEach(() => vi.useRealTimers());

	function runtime() {
		const listeners: ((m: unknown) => void)[] = [];
		return {
			sendMessage: vi.fn(),
			onMessage: { addListener: (l: (m: unknown) => void) => listeners.push(l) },
			deliver: (m: unknown) => listeners.forEach((l) => l(m))
		};
	}

	it('sends each report once and again when it changes', () => {
		const doc = page('<p/>', '(1) Chat');
		const rt = runtime();
		let tick = () => {};
		start(doc, rt, {
			hostname: 'teams.cloud.microsoft',
			now: () => 0,
			setInterval: ((f: () => void) => (tick = f)) as never
		});
		expect(rt.sendMessage).toHaveBeenCalledTimes(2);
		tick();
		expect(rt.sendMessage).toHaveBeenCalledTimes(2);
		doc.title = '(2) Chat';
		tick();
		expect(rt.sendMessage).toHaveBeenLastCalledWith({ report: { type: 'unread', count: 2 } });
	});

	it('clicks on request in the top frame only', () => {
		const doc = page('<button data-tid="hangup-main-btn"></button>');
		const click = vi.fn();
		doc.querySelector('button')!.addEventListener('click', click);
		const rt = runtime();
		start(doc, rt, { hostname: 'teams.cloud.microsoft', setInterval: (() => 0) as never });
		rt.deliver({ type: 'do', action: 'hangUp' });
		rt.deliver({ nonsense: true });
		expect(click).toHaveBeenCalledTimes(window === window.top ? 1 : 0);
	});
});
