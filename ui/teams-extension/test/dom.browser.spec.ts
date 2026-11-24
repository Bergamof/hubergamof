import { afterEach, describe, expect, it, vi } from 'vitest';
import type { FrameContext } from '../lib/teams-dom.js';
import { RESEND_AFTER_MS, start } from '../lib/content-main.js';
import {
	avatarIcon,
	frameRole,
	openChannel,
	openConversation,
	openPanel,
	performAction,
	readBadges,
	readUnreadChannels,
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

// The markup of Teams, reduced: the first reference of `aria-labelledby` is
// the "Non lu" label, the name is in the title span.
function conversation(name: string, unread: boolean, type = 'chat') {
	return `<div role="treeitem" aria-level="2" data-item-type="${type}"
		data-fui-tree-item-value="id:${name}" aria-labelledby="chat_list_unread_text title-${name}">
		<span class="fui-Avatar"><img class="fui-Avatar__image" src="data:,"></span>
		${unread ? '<div data-tid="unread" id="unread-label">Non lu</div>' : ''}
		<span id="title-${name}" role="text">${name}</span></div>`;
}

const CONVERSATIONS =
	conversation('Alice', true) + conversation('Bob', false) + conversation('Carol', true);

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
	it('reads the duration Teams shows', () => {
		const call = readCall(
			page(
				'<button data-tid="hangup-main-btn"></button><span data-tid="call-duration">12:34</span>'
			)
		);
		expect(call).toMatchObject({ phase: 'inCall', elapsedSeconds: 754 });
	});
	it('has no duration when Teams shows none or something else', () => {
		expect(
			readCall(page('<button data-tid="hangup-main-btn"></button>')).elapsedSeconds
		).toBeNull();
		expect(
			readCall(page('<span data-tid="call-duration">en cours</span>')).elapsedSeconds
		).toBeNull();
	});
	it('sees the pre-join screen', () => {
		expect(readCall(page('<div data-tid="calling-prejoin-screen"></div>')).phase).toBe('prejoin');
	});
});

describe('readUnreadConversations', () => {
	it('names the unread conversations only', () => {
		expect(readUnreadConversations(page(CONVERSATIONS))?.map((c) => c.name)).toEqual([
			'Alice',
			'Carol'
		]);
	});
	it('reads the title, not the "Non lu" label', () => {
		expect(readUnreadConversations(page(conversation('Équipe Exposition', true)))).toEqual([
			{ name: 'Équipe Exposition', icon: null, id: 'id:Équipe Exposition' }
		]);
	});
	it('leaves the channels of the Teams view out and learns nothing from them alone', () => {
		const channels = conversation('Général', true, 'channel');
		expect(readUnreadConversations(page(channels))).toBeNull();
		const both = conversation('Alice', true) + channels;
		expect(readUnreadConversations(page(both))?.map((c) => c.name)).toEqual(['Alice']);
	});
	it('learns nothing when the list is not on screen', () => {
		expect(readUnreadConversations(page('<p>Calendrier</p>'))).toBeNull();
	});
	it('skips an entry without a name', () => {
		const doc = page(
			'<div role="treeitem" aria-level="2" data-item-type="chat"><i data-tid="unread"></i></div>'
		);
		expect(readUnreadConversations(doc)).toEqual([]);
	});
});

describe('readUnreadChannels', () => {
	const channels =
		conversation('Général', true, 'channel') + conversation('Annonces', false, 'channel');

	it('reads the unread channels, without avatar', () => {
		expect(readUnreadChannels(page(channels))).toEqual([
			{ name: 'Général', icon: null, id: 'id:Général' }
		]);
	});
	it('leaves the conversations out and learns nothing from them alone', () => {
		expect(readUnreadChannels(page(CONVERSATIONS))).toBeNull();
		const both = CONVERSATIONS + channels;
		expect(readUnreadChannels(page(both))?.map((c) => c.name)).toEqual(['Général']);
		expect(readUnreadConversations(page(both))?.map((c) => c.name)).toEqual(['Alice', 'Carol']);
	});
	it('is part of what Teams reports, once a channel list is on screen', () => {
		const withList = page(
			railItem(ACTIVITY_APP, '0') + railItem(CHAT_RAIL, '0') + channels,
			'(1) Teams'
		);
		const reports = readReports(withList, 'teams.cloud.microsoft', 0, TOP);
		expect(reports.find((r) => r.type === 'channels')).toMatchObject({
			unread: [{ name: 'Général' }]
		});
		const without = page(railItem(ACTIVITY_APP, '0') + railItem(CHAT_RAIL, '0'), '(1) Teams');
		const types = readReports(without, 'teams.cloud.microsoft', 0, TOP).map((r) => r.type);
		expect(types).not.toContain('channels');
	});
});

describe('openChannel', () => {
	it('clicks the channel of that identifier, and no conversation', async () => {
		const doc = page(conversation('Général', true) + conversation('Général', true, 'channel'));
		const clicked: (string | null)[] = [];
		for (const item of doc.querySelectorAll('[role="treeitem"]')) {
			item.addEventListener('click', () => clicked.push(item.getAttribute('data-item-type')));
		}
		expect(await openChannel(doc, 'id:Général')).toBe(true);
		expect(clicked).toEqual(['channel']);
	});
	it('goes to the Teams application first when the list is not on screen', async () => {
		const doc = page(`<button data-tid="${TEAMS_RAIL}"></button>`);
		doc.querySelector('button')!.addEventListener('click', () => {
			doc.body.insertAdjacentHTML('beforeend', conversation('Général', true, 'channel'));
		});
		expect(await openChannel(doc, 'id:Général', async () => {})).toBe(true);
	});
	it('does not take the identifier for a selector, nor open a missing channel', async () => {
		const doc = page(conversation('Général', true, 'channel'));
		expect(await openChannel(doc, 'x"] , [role="treeitem')).toBe(false);
		expect(await openChannel(page('<p/>'), 'id:Général', async () => {})).toBe(false);
	});
});

// A 2×2 opaque PNG.
const AVATAR_PNG =
	'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFklEQVR4nGP8z8Dwn4GBgYGJAQoAHxQCAZlgAXUAAAAASUVORK5CYII=';
const PNG_SIGNATURE_BASE64 = 'iVBORw0KGgo';

async function loadedAvatar(source: string) {
	const image = new Image();
	image.src = source;
	await image.decode();
	return image;
}

describe('avatars', () => {
	afterEach(() => {
		document.body.innerHTML = '';
	});

	it('sends the pixels of a loaded avatar as an icon', async () => {
		const icon = avatarIcon(document, await loadedAvatar(AVATAR_PNG));
		expect(icon?.startsWith(PNG_SIGNATURE_BASE64)).toBe(true);
	});

	it('goes with its conversation, and a conversation without avatar has none', async () => {
		document.body.innerHTML =
			conversation('Alice', true).replace('src="data:,"', `src="${AVATAR_PNG}"`) +
			conversation('Bob', true).replace(/<img[^>]*>/, '');
		await document.querySelector<HTMLImageElement>('img')!.decode();
		const [alice, bob] = readUnreadConversations(document)!;
		expect(alice.icon?.startsWith(PNG_SIGNATURE_BASE64)).toBe(true);
		expect(bob).toEqual({ name: 'Bob', icon: null, id: 'id:Bob' });
	});

	it('gives none while the image is not loaded', () => {
		document.body.innerHTML = conversation('Alice', true);
		expect(readUnreadConversations(document)).toEqual([
			{ name: 'Alice', icon: null, id: 'id:Alice' }
		]);
	});
});

describe('openConversation', () => {
	const CHAT_APP = '86fcd49b-61a2-4701-b771-54728cd291fb';

	it('clicks the conversation of that identifier, and no other', async () => {
		const doc = page(conversation('Alice', true) + conversation('Bob', true));
		const clicked: string[] = [];
		for (const item of doc.querySelectorAll('[role="treeitem"]')) {
			item.addEventListener('click', () =>
				clicked.push(item.getAttribute('data-fui-tree-item-value')!)
			);
		}
		expect(await openConversation(doc, 'id:Bob')).toBe(true);
		expect(clicked).toEqual(['id:Bob']);
	});

	it('does not take the identifier for a selector', async () => {
		const doc = page(conversation('Alice', true));
		expect(await openConversation(doc, 'x"] , [role="treeitem')).toBe(false);
	});

	it('refuses a conversation that is not there, after trying the rail', async () => {
		const doc = page(`<button data-tid="${CHAT_APP}"></button>`);
		const wait = vi.fn(async () => {});
		expect(await openConversation(doc, 'id:Alice', wait)).toBe(false);
		expect(wait).toHaveBeenCalled();
	});

	it('goes to the Chat application first when the list is not on screen', async () => {
		const doc = page(`<button data-tid="${CHAT_APP}"></button>`);
		const item = () => doc.querySelector('[role="treeitem"]');
		doc.querySelector('button')!.addEventListener('click', () => {
			doc.body.insertAdjacentHTML('beforeend', conversation('Alice', true));
		});
		const opened = vi.fn();
		const wait = async () => {
			item()?.addEventListener('click', opened, { once: true });
		};
		expect(await openConversation(doc, 'id:Alice', wait)).toBe(true);
		expect(item()).not.toBeNull();
	});

	it('does not touch the rail when the conversation is already listed', async () => {
		const doc = page(`<button data-tid="${CHAT_APP}"></button>` + conversation('Alice', true));
		const rail = vi.fn();
		doc.querySelector('button')!.addEventListener('click', rail);
		expect(await openConversation(doc, 'id:Alice')).toBe(true);
		expect(rail).not.toHaveBeenCalled();
	});
});

// The rail of Teams, reduced: the counter is a sibling of the button.
const ACTIVITY_APP = '14d6962d-6eeb-4f48-8890-de55454bb136';
const CHAT_RAIL = '86fcd49b-61a2-4701-b771-54728cd291fb';
const TEAMS_RAIL = '2a84919f-59d8-4441-a975-2a8c2643b741';
function railItem(tid: string, badge?: string) {
	return `<div role="none"><button data-tid="${tid}"></button>${
		badge === undefined ? '' : `<div class="fui-Badge fui-CounterBadge">${badge}</div>`
	}<div>1 conversation avec de nouveaux messages</div></div>`;
}

describe('readBadges', () => {
	it('reads the counters of the bell and the bubble', () => {
		const doc = page(railItem(ACTIVITY_APP, '2') + railItem(CHAT_RAIL, '3'));
		expect(readBadges(doc)).toEqual({ type: 'badges', activity: 2, chats: 3, teams: 0 });
	});
	it('counts a button without badge as zero', () => {
		const doc = page(railItem(ACTIVITY_APP) + railItem(CHAT_RAIL, '1'));
		expect(readBadges(doc)).toEqual({ type: 'badges', activity: 0, chats: 1, teams: 0 });
	});
	it('caps "99+" and ignores the text around the digits', () => {
		const doc = page(railItem(ACTIVITY_APP, '99+') + railItem(CHAT_RAIL, '12'));
		expect(readBadges(doc)).toEqual({ type: 'badges', activity: 99, chats: 12, teams: 0 });
	});
	it('reads the counter of the Teams application, zero when it is not pinned', () => {
		const rail = railItem(ACTIVITY_APP, '2') + railItem(CHAT_RAIL, '3');
		expect(readBadges(page(rail + railItem(TEAMS_RAIL, '4')))).toEqual({
			type: 'badges',
			activity: 2,
			chats: 3,
			teams: 4
		});
		expect(readBadges(page(rail))).toMatchObject({ teams: 0 });
	});
	it('learns nothing when the rail is not on screen', () => {
		expect(readBadges(page('<p/>'))).toBeNull();
		expect(readBadges(page(railItem(CHAT_RAIL, '1')))).toBeNull();
	});
	it('is part of what Teams reports', () => {
		const doc = page(railItem(ACTIVITY_APP, '1') + railItem(CHAT_RAIL, '1'), '(2) Chat');
		const types = readReports(doc, 'teams.cloud.microsoft', 0, TOP).map((r) => r.type);
		expect(types).toContain('badges');
	});
});

describe('openPanel', () => {
	it('clicks the button of the panel, and no other', () => {
		const doc = page(railItem(ACTIVITY_APP, '1') + railItem(CHAT_RAIL, '1'));
		const clicked: string[] = [];
		for (const button of doc.querySelectorAll('button')) {
			button.addEventListener('click', () => clicked.push(button.dataset.tid!));
		}
		expect(openPanel(doc, 'activity')).toBe(true);
		expect(openPanel(doc, 'chat')).toBe(true);
		expect(clicked).toEqual([ACTIVITY_APP, CHAT_RAIL]);
		doc.body.insertAdjacentHTML('beforeend', railItem(TEAMS_RAIL, '1'));
		for (const button of doc.querySelectorAll('button')) {
			button.addEventListener('click', () => clicked.push(button.dataset.tid!));
		}
		expect(openPanel(doc, 'teams')).toBe(true);
		expect(clicked.at(-1)).toBe(TEAMS_RAIL);
	});
	it('refuses an unknown panel and a missing button', () => {
		expect(openPanel(page(railItem(CHAT_RAIL)), 'activity')).toBe(false);
		expect(openPanel(page(railItem(CHAT_RAIL)), 'settings')).toBe(false);
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

const TOP: FrameContext = { top: true, underTeams: false };
const CALENDAR: FrameContext = { top: false, underTeams: true };

describe('readReports', () => {
	it('reports the count, the names and the call of Teams', () => {
		const doc = page(CONVERSATIONS, '(2) Chat');
		expect(readReports(doc, 'teams.cloud.microsoft', 0, TOP).map((r) => r.type)).toEqual([
			'unread',
			'conversations',
			'call'
		]);
	});
	it('says nobody is unread without reading the list when the count is zero', () => {
		const reports = readReports(page(CONVERSATIONS, 'Chat'), 'teams.cloud.microsoft', 0, TOP);
		expect(reports[1]).toEqual({ type: 'conversations', unread: [] });
	});
	it('keeps the previous names when the list is hidden', () => {
		const reports = readReports(page('<p/>', '(2) Calendrier'), 'teams.cloud.microsoft', 0, TOP);
		expect(reports.map((r) => r.type)).toEqual(['unread', 'call']);
	});
	it('reads only meetings in the calendar frame', () => {
		const doc = page(
			'<div role="button" aria-label="X, 10:00 à 11:00, lundi 17 novembre 2026"></div>'
		);
		expect(readReports(doc, 'outlook.office.com', 0, CALENDAR).map((r) => r.type)).toEqual([
			'meetings'
		]);
	});
	it('ignores a Teams sub-frame and Outlook outside of Teams', () => {
		const sub = { top: false, underTeams: false };
		expect(readReports(page('<p/>', '(1) x'), 'teams.cloud.microsoft', 0, sub)).toEqual([]);
		const slot = '<div role="button" aria-label="X, 10:00 à 11:00, lundi 17 novembre 2026"></div>';
		expect(readReports(page(slot), 'outlook.office.com', 0, TOP)).toEqual([]);
	});
	it('reads nothing elsewhere', () => {
		expect(readReports(page('<p/>'), 'example.org', 0, TOP)).toEqual([]);
		expect(frameRole('example.org', TOP)).toBe('other');
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
	it('enlarges the call from its compact view, and does nothing without it', () => {
		const doc = page(
			'<button data-tid="call-monitor-navigate-cw-button" aria-label="Agrandir la fenêtre de réunion"></button>'
		);
		const click = vi.fn();
		doc.querySelector('button')!.addEventListener('click', click);
		expect(performAction(doc, 'returnToCall')).toBe(true);
		expect(click).toHaveBeenCalledOnce();
		expect(performAction(page('<p/>'), 'returnToCall')).toBe(false);
	});
	it('does nothing for a missing control or an unsupported action', () => {
		const doc = page('<p/>');
		expect(performAction(doc, 'hangUp')).toBe(false);
		expect(performAction(doc, 'accept')).toBe(false);
		expect(performAction(doc, 'constructor')).toBe(false);
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
			context: TOP,
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

	it('sends everything again after a while, for a worker that forgot it', () => {
		const rt = runtime();
		let tick = () => {};
		let time = 0;
		start(page('<p/>', '(1) Chat'), rt, {
			hostname: 'teams.cloud.microsoft',
			context: TOP,
			now: () => time,
			setInterval: ((f: () => void) => (tick = f)) as never
		});
		expect(rt.sendMessage).toHaveBeenCalledTimes(2);
		time = RESEND_AFTER_MS - 1;
		tick();
		expect(rt.sendMessage).toHaveBeenCalledTimes(2);
		time = RESEND_AFTER_MS;
		tick();
		expect(rt.sendMessage).toHaveBeenCalledTimes(4);
	});

	it('does not report again each second the duration of a call moves', () => {
		const doc = page(
			'<button data-tid="hangup-main-btn"></button><span data-tid="call-duration">0:10</span>'
		);
		const duration = doc.querySelector('[data-tid="call-duration"]')!;
		let time = 0;
		let tick = () => {};
		const rt = runtime();
		start(doc, rt, {
			hostname: 'teams.cloud.microsoft',
			context: TOP,
			now: () => time,
			setInterval: ((f: () => void) => (tick = f)) as never
		});
		const calls = () =>
			rt.sendMessage.mock.calls.filter(([message]) => message.report.type === 'call');
		expect(calls()).toHaveLength(1);
		duration.textContent = '0:12';
		time = 2000;
		tick();
		expect(calls()).toHaveLength(1);
		// Past the resend delay the report goes again, with the duration to set the count right.
		duration.textContent = '0:40';
		time = RESEND_AFTER_MS;
		tick();
		expect(calls()).toHaveLength(2);
		expect(calls()[1][0].report.elapsedSeconds).toBe(40);
	});

	it('does not click in a sub-frame', () => {
		const doc = page('<button data-tid="hangup-main-btn"></button>');
		const click = vi.fn();
		doc.querySelector('button')!.addEventListener('click', click);
		const rt = runtime();
		start(doc, rt, {
			hostname: 'teams.cloud.microsoft',
			context: { top: false, underTeams: false },
			setInterval: (() => 0) as never
		});
		rt.deliver({ type: 'do', action: 'hangUp' });
		expect(click).not.toHaveBeenCalled();
	});

	it('shows a panel of the rail on request, in the top frame only', () => {
		const doc = page(railItem(ACTIVITY_APP, '1'));
		const click = vi.fn();
		doc.querySelector('button')!.addEventListener('click', click);
		const sub = runtime();
		start(doc, sub, {
			hostname: 'teams.cloud.microsoft',
			context: { top: false, underTeams: false },
			setInterval: (() => 0) as never
		});
		sub.deliver({ type: 'openPanel', panel: 'activity' });
		expect(click).not.toHaveBeenCalled();
		const top = runtime();
		start(doc, top, {
			hostname: 'teams.cloud.microsoft',
			context: TOP,
			setInterval: (() => 0) as never
		});
		top.deliver({ type: 'openPanel', panel: 'activity' });
		top.deliver({ type: 'openPanel', panel: 7 });
		expect(click).toHaveBeenCalledOnce();
	});

	it('clicks on request in the top frame only', () => {
		const doc = page('<button data-tid="hangup-main-btn"></button>');
		const click = vi.fn();
		doc.querySelector('button')!.addEventListener('click', click);
		const rt = runtime();
		start(doc, rt, {
			hostname: 'teams.cloud.microsoft',
			context: TOP,
			setInterval: (() => 0) as never
		});
		rt.deliver({ type: 'do', action: 'hangUp' });
		rt.deliver({ nonsense: true });
		expect(click).toHaveBeenCalledOnce();
	});
});
