// What the extension reads in Teams and the clicks it makes. Teams has no
// public hook: everything here rests on its current markup (see the Teams
// section of docs/integrations-desktop.md for the survey and the way to
// check it again when Teams changes).

import { cameraOnFromLabel, mutedFromLabel } from './labels.js';
import { meetingFromLabel } from './meetings.js';
import { unreadCountFromTitle } from './title.js';

const HANGUP = '[data-tid="hangup-main-btn"], #hangup-button';
const MICROPHONE = '#microphone-button';
const CAMERA = '#video-button';
const PREJOIN = '[data-tid="calling-prejoin-screen"]';
const PREJOIN_MICROPHONE = 'input[data-tid="toggle-mute"]';
const PREJOIN_CAMERA = 'input[data-tid="toggle-video"]';
const CALL_DURATION = '[data-tid="call-duration"]';
/** Enlarges the call from its compact view (labelled "Agrandir la fenêtre de réunion"). */
const RETURN_TO_CALL = '[data-tid="call-monitor-navigate-cw-button"]';
// Conversations only: the channels of the Teams view are `data-item-type="channel"`.
const CONVERSATION = '[role="treeitem"][aria-level="2"][data-item-type="chat"]';
const UNREAD_MARK = '[data-tid="unread"]';
const CONVERSATION_TITLE = '[role="text"][id^="title-"]';
const AVATAR = 'img.fui-Avatar__image';
/** Attribute of a conversation holding its identifier in Teams, opaque. */
const CONVERSATION_ID = 'data-fui-tree-item-value';
/** The Chat application of the left rail: a constant of Teams, in any language. */
const CHAT_APP = '[data-tid="86fcd49b-61a2-4701-b771-54728cd291fb"]';
/** The Activity application of the rail (the bell): a constant of Teams too. */
const ACTIVITY_APP = '[data-tid="14d6962d-6eeb-4f48-8890-de55454bb136"]';
/** The counter of a rail application, next to its button, in the same wrapper. */
const RAIL_BADGE = '.fui-CounterBadge';
/** Panels of the rail the manager may ask for, by name. */
const PANELS = new Map([
	['activity', ACTIVITY_APP],
	['chat', CHAT_APP]
]);
/** Largest counter reported (the badge says "99+" past 99). */
const MAX_BADGE = 9999;
const CALENDAR_SLOT = 'div[role="button"][aria-label]';

/** Most unread names and meetings one report may hold (the manager keeps fewer). */
const MAX_ITEMS = 100;

/** Side of an avatar sent to the manager, in pixels. */
const ICON_SIZE = 64;

/**
 * Longest avatar sent, once in base64: a report holds one per conversation and
 * stays far under the 1 MiB the browser allows a message to the host.
 */
const MAX_ICON_BASE64 = 16 * 1024;

/** How long to wait for the conversation list after a click on the rail. */
const LIST_ATTEMPTS = 20;
const LIST_POLL_MS = 100;

/** Avatars already encoded, by image element, until Teams changes their source. */
const ICONS = new WeakMap();

/** The Outlook frame that holds the calendar, and Teams itself. */
export const FrameRole = Object.freeze({ Teams: 'teams', Calendar: 'calendar', Other: 'other' });

/**
 * What a frame is for. Teams only counts as the top frame of its tab, the
 * calendar only when Teams itself embeds it (the mail tabs of Outlook are
 * never read).
 * @typedef {{ top: boolean, underTeams: boolean }} FrameContext
 * @param {string} hostname
 * @param {FrameContext} context
 */
export function frameRole(hostname, context) {
	if (hostname === 'teams.cloud.microsoft' && context.top) return FrameRole.Teams;
	if (hostname === 'outlook.office.com' && context.underTeams) return FrameRole.Calendar;
	return FrameRole.Other;
}

/**
 * @param {ParentNode} root
 * @param {string} selector
 */
const labelOf = (root, selector) => root.querySelector(selector)?.getAttribute('aria-label');

/**
 * The state of the call. A ringing call is not detected yet (its markup was
 * never observed).
 * @param {Document} doc
 * @returns {{ type: 'call', phase: 'idle' | 'prejoin' | 'inCall', muted: boolean | null,
 *   cameraOn: boolean | null, caller: null }}
 */
export function readCall(doc) {
	const idle = { type: /** @type {const} */ ('call'), phase: /** @type {const} */ ('idle') };
	const base = { ...idle, muted: null, cameraOn: null, caller: null };
	if (doc.querySelector(PREJOIN)) return { ...base, phase: 'prejoin' };
	if (!doc.querySelector(HANGUP) && !doc.querySelector(CALL_DURATION)) return base;
	return {
		...base,
		phase: 'inCall',
		muted: mutedFromLabel(labelOf(doc, MICROPHONE)),
		cameraOn: cameraOnFromLabel(labelOf(doc, CAMERA))
	};
}

/**
 * The unread conversations, with their avatar; `null` when the conversation
 * list is not on screen (it only is on the Conversation view; the Teams view
 * lists channels, which are left out), so that nothing is learnt.
 * @param {Document} doc
 * @returns {{ name: string, icon: string | null, id: string | null }[] | null}
 */
export function readUnreadConversations(doc) {
	const items = [...doc.querySelectorAll(CONVERSATION)];
	if (items.length === 0) return null;
	return items
		.filter((item) => item.querySelector(UNREAD_MARK))
		.map((item) => ({
			name: conversationName(item),
			icon: conversationIcon(doc, item),
			id: item.getAttribute(CONVERSATION_ID)
		}))
		.filter((conversation) => conversation.name !== '')
		.slice(0, MAX_ITEMS);
}

/** @param {Element} item */
function conversationName(item) {
	return item.querySelector(CONVERSATION_TITLE)?.textContent?.trim() ?? '';
}

/**
 * The avatar of a conversation as base64, `null` when it has none, is not
 * loaded yet or cannot be read.
 * @param {Document} doc
 * @param {Element} item
 */
function conversationIcon(doc, item) {
	const image = item.querySelector(AVATAR);
	if (!(image instanceof HTMLImageElement) || !image.complete || image.naturalWidth === 0) {
		return null;
	}
	const cached = ICONS.get(image);
	if (cached?.source === image.currentSrc) return cached.icon;
	const icon = avatarIcon(doc, image);
	ICONS.set(image, { source: image.currentSrc, icon });
	return icon;
}

/**
 * `image`, square-cropped and scaled to the size of an icon, as base64 (PNG,
 * or JPEG when the PNG is too big); `null` when the browser refuses to read it
 * or it stays too big. Only pixels leave the page, never the address of the
 * image, which carries names.
 * @param {Document} doc
 * @param {HTMLImageElement} image
 */
export function avatarIcon(doc, image) {
	const canvas = doc.createElement('canvas');
	canvas.width = ICON_SIZE;
	canvas.height = ICON_SIZE;
	const context = canvas.getContext('2d');
	if (!context) return null;
	const side = Math.min(image.naturalWidth, image.naturalHeight);
	const left = (image.naturalWidth - side) / 2;
	const top = (image.naturalHeight - side) / 2;
	try {
		context.drawImage(image, left, top, side, side, 0, 0, ICON_SIZE, ICON_SIZE);
		for (const [type, quality] of /** @type {const} */ ([
			['image/png', undefined],
			['image/jpeg', 0.85]
		])) {
			const url = canvas.toDataURL(type, quality);
			const base64 = url.slice(url.indexOf(',') + 1);
			if (base64.length <= MAX_ICON_BASE64) return base64;
		}
	} catch {
		// A tainted canvas: the image is not readable from this page.
	}
	return null;
}

/**
 * Meetings of the calendar that are not over yet, soonest first; `null` when
 * the page holds no meeting at all (the calendar is not showing).
 * @param {Document} doc
 * @param {number} nowSeconds
 */
export function readMeetings(doc, nowSeconds) {
	const meetings = [...doc.querySelectorAll(CALENDAR_SLOT)]
		.map((slot) => meetingFromLabel(slot.getAttribute('aria-label') ?? ''))
		.filter((meeting) => meeting !== null);
	if (meetings.length === 0) return null;
	return meetings
		.filter((meeting) => meeting.endsAt > nowSeconds)
		.sort((a, b) => a.startsAt - b.startsAt)
		.slice(0, MAX_ITEMS);
}

/**
 * What the badge next to the rail button `selector` counts: `0` without
 * badge, `null` when the button is not on screen.
 * @param {Document} doc
 * @param {string} selector
 */
function railCount(doc, selector) {
	const button = doc.querySelector(selector);
	if (!button) return null;
	const digits = /\d+/.exec(button.parentElement?.querySelector(RAIL_BADGE)?.textContent ?? '');
	return digits ? Math.min(Number(digits[0]), MAX_BADGE) : 0;
}

/**
 * The counters of the Activity and Conversation applications of the rail;
 * `null` when the rail is not on screen (the previous ones then stay).
 * @param {Document} doc
 */
export function readBadges(doc) {
	const activity = railCount(doc, ACTIVITY_APP);
	const chats = railCount(doc, CHAT_APP);
	return activity === null || chats === null ? null : { type: 'badges', activity, chats };
}

/**
 * Every report this frame can make now, in the manager's own format.
 * @param {Document} doc
 * @param {string} hostname
 * @param {number} nowSeconds
 * @param {FrameContext} context
 */
export function readReports(doc, hostname, nowSeconds, context) {
	const role = frameRole(hostname, context);
	if (role === FrameRole.Calendar) {
		const items = readMeetings(doc, nowSeconds);
		return items ? [{ type: 'meetings', items }] : [];
	}
	if (role !== FrameRole.Teams) return [];
	const count = unreadCountFromTitle(doc.title);
	const names = count === 0 ? [] : readUnreadConversations(doc);
	const badges = readBadges(doc);
	return [
		{ type: 'unread', count },
		...(badges ? [badges] : []),
		...(names ? [{ type: 'conversations', unread: names }] : []),
		readCall(doc)
	];
}

/**
 * @param {Document} doc
 * @param {string} id
 */
function findConversation(doc, id) {
	// Compared with the attribute, never put in a selector.
	return [...doc.querySelectorAll(CONVERSATION)].find(
		(item) => item.getAttribute(CONVERSATION_ID) === id
	);
}

/**
 * Shows the conversation `id` (one the extension reported): clicks it in the
 * list, after a click on the Chat application of the rail when the list is not
 * on screen (the Teams or the calendar view). Only clicks, never navigates: a
 * reload would end a call in progress. Whether it was found.
 * @param {Document} doc
 * @param {string} id
 * @param {(ms: number) => Promise<void>} [wait]
 */
export async function openConversation(
	doc,
	id,
	wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
) {
	if (!findConversation(doc, id)) {
		const rail = doc.querySelector(CHAT_APP);
		if (rail instanceof HTMLElement) rail.click();
		for (let attempt = 0; attempt < LIST_ATTEMPTS && !findConversation(doc, id); attempt++) {
			await wait(LIST_POLL_MS);
		}
	}
	const item = findConversation(doc, id);
	if (!(item instanceof HTMLElement)) return false;
	item.click();
	return true;
}

/**
 * Shows `panel` of the rail (`activity` or `chat`) by clicking its button,
 * which already being the open panel leaves as it is. Whether the button was
 * there.
 * @param {Document} doc
 * @param {string} panel
 */
export function openPanel(doc, panel) {
	const selector = PANELS.get(panel);
	const button = selector ? doc.querySelector(selector) : null;
	if (!(button instanceof HTMLElement)) return false;
	button.click();
	return true;
}

/**
 * Clicks the control that does `action`; whether it was there to click.
 * Accepting and declining an incoming call are not supported yet.
 * @param {Document} doc
 * @param {string} action
 */
export function performAction(doc, action) {
	const selectors = new Map([
		['toggleMute', [MICROPHONE, PREJOIN_MICROPHONE]],
		['toggleCamera', [CAMERA, PREJOIN_CAMERA]],
		['hangUp', [HANGUP]],
		['returnToCall', [RETURN_TO_CALL]]
	]);
	const candidates = selectors.get(action) ?? [];
	for (const selector of candidates) {
		const control = doc.querySelector(selector);
		if (control instanceof HTMLElement) {
			control.click();
			return true;
		}
	}
	return false;
}
