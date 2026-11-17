import { Relay } from './lib/relay.js';

const NATIVE_HOST = 'com.hubergamof.teams';
const RETRY_ALARM = 'reconnect';
const RETRY_PERIOD_MINUTES = 0.5;
const TEAMS_URLS = ['https://teams.cloud.microsoft/*'];

/** Tab that sent the latest call report: where the call buttons are clicked. */
let callTab = null;

async function teamsTab() {
	if (callTab !== null) return callTab;
	const [tab] = await chrome.tabs.query({ url: TEAMS_URLS });
	return tab?.id ?? null;
}

async function forwardToTeams(request) {
	const tab = await teamsTab();
	if (tab === null || request?.type !== 'do') return;
	// Frame 0: Teams itself, not the calendar frame.
	chrome.tabs.sendMessage(tab, request, { frameId: 0 }).catch(() => {
		callTab = null;
	});
}

const relay = new Relay({
	connect: () => chrome.runtime.connectNative(NATIVE_HOST),
	now: Date.now,
	onRequest: forwardToTeams
});

chrome.runtime.onMessage.addListener((message, sender) => {
	if (!message?.report) return;
	if (message.report.type === 'call') callTab = sender.tab?.id ?? null;
	relay.report(message.report);
});

// A suspended worker wakes up for these: link again when the manager is back.
chrome.alarms.create(RETRY_ALARM, { periodInMinutes: RETRY_PERIOD_MINUTES });
chrome.alarms.onAlarm.addListener(() => relay.connect());
chrome.runtime.onStartup.addListener(() => relay.connect());
chrome.runtime.onInstalled.addListener(() => relay.connect());
