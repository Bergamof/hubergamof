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
const CONVERSATION = '[role="treeitem"][aria-level="2"]';
const UNREAD_MARK = '[data-tid="unread"]';
const CALENDAR_SLOT = 'div[role="button"][aria-label]';

/** Most unread names and meetings one report may hold (the manager keeps fewer). */
const MAX_ITEMS = 100;

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
 * Names of the unread conversations; `null` when the conversation list is not
 * on screen (it only is on the Conversation view), so that nothing is learnt.
 * @param {Document} doc
 * @returns {string[] | null}
 */
export function readUnreadConversations(doc) {
	const items = [...doc.querySelectorAll(CONVERSATION)];
	if (items.length === 0) return null;
	return items
		.filter((item) => item.querySelector(UNREAD_MARK))
		.map((item) => conversationName(doc, item))
		.filter((name) => name !== '')
		.slice(0, MAX_ITEMS);
}

/**
 * @param {Document} doc
 * @param {Element} item
 */
function conversationName(doc, item) {
	const ids = item.getAttribute('aria-labelledby')?.split(/\s+/) ?? [];
	const named = ids.map((id) => doc.getElementById(id)?.textContent?.trim()).find(Boolean);
	return named ?? '';
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
	return [
		{ type: 'unread', count },
		...(names ? [{ type: 'conversations', unread: names }] : []),
		readCall(doc)
	];
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
		['hangUp', [HANGUP]]
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
