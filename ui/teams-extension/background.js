import { Relay } from './lib/relay.js';

const NATIVE_HOST = 'com.hubergamof.teams';
const RETRY_ALARM = 'reconnect';
const RETRY_PERIOD_MINUTES = 0.5;
/** What the manager may ask; anything else is dropped before it reaches a page. */
const ACTIONS = new Set(['toggleMute', 'toggleCamera', 'hangUp']);
const TEAMS_ORIGIN = 'https://teams.cloud.microsoft';
const CALENDAR_ORIGIN = 'https://outlook.office.com';
const TEAMS_URLS = ['https://teams.cloud.microsoft/*'];

/** Tab that sent the latest call report: where the call buttons are clicked. */
let callTab = null;

async function teamsTab() {
	if (callTab !== null) return callTab;
	const [tab] = await chrome.tabs.query({ url: TEAMS_URLS });
	return tab?.id ?? null;
}

async function forwardToTeams(request) {
	if (request?.type !== 'do' || !ACTIONS.has(request.action)) return;
	const tab = await teamsTab();
	if (tab === null) return;
	// Frame 0: Teams itself, not the calendar frame.
	chrome.tabs.sendMessage(tab, { type: 'do', action: request.action }, { frameId: 0 }).catch(() => {
		callTab = null;
	});
}

const relay = new Relay({
	connect: () => chrome.runtime.connectNative(NATIVE_HOST),
	now: Date.now,
	onRequest: forwardToTeams
});

/** Which frame may report what: Teams itself its own state, the calendar the meetings. */
function mayReport(report, sender) {
	if (sender.id !== chrome.runtime.id) return false;
	if (report.type === 'meetings') return sender.origin === CALENDAR_ORIGIN;
	return sender.origin === TEAMS_ORIGIN && sender.frameId === 0;
}

chrome.runtime.onMessage.addListener((message, sender) => {
	if (!message?.report || !mayReport(message.report, sender)) return;
	if (message.report.type === 'call') callTab = sender.tab?.id ?? null;
	relay.report(message.report);
});

// A suspended worker wakes up for these: link again when the manager is back.
chrome.alarms.create(RETRY_ALARM, { periodInMinutes: RETRY_PERIOD_MINUTES });
chrome.alarms.onAlarm.addListener(() => relay.connect());
chrome.runtime.onStartup.addListener(() => relay.connect());
chrome.runtime.onInstalled.addListener(() => relay.connect());
