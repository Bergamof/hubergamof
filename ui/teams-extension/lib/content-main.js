import { openConversation, performAction, readReports } from './teams-dom.js';

/** Teams changes its page without events we could rely on: look again regularly. */
export const POLL_INTERVAL_MS = 2000;

/**
 * A report is sent again after this long even if unchanged: the extension's
 * worker is suspended and restarted by the browser, and forgets what it was
 * told, which the page would not repeat on its own.
 */
export const RESEND_AFTER_MS = 30_000;

/**
 * Reports what the page shows, whenever it changes (and now and then even if
 * it does not, see {@link RESEND_AFTER_MS}), and carries out the
 * actions the manager asks for.
 * @param {Document} doc
 * @param {{ sendMessage: (message: unknown) => unknown,
 *   onMessage: { addListener: (listener: (message: any) => void) => void } }} runtime
 * @param {{ hostname?: string, context?: import('./teams-dom.js').FrameContext, now?: () => number, setInterval?: typeof setInterval }} [env]
 */
export function start(doc, runtime, env = {}) {
	const hostname = env.hostname ?? doc.location.hostname;
	const view = doc.defaultView;
	const context = env.context ?? {
		top: view === view?.top,
		underTeams: doc.location.ancestorOrigins?.contains('https://teams.cloud.microsoft') ?? false
	};
	const now = env.now ?? Date.now;
	const schedule = env.setInterval ?? setInterval;
	/** Last report sent of each type and when, to send changes (and reminders) only. */
	const sent = new Map();

	function report() {
		const time = now();
		for (const item of readReports(doc, hostname, Math.floor(time / 1000), context)) {
			const text = JSON.stringify(item);
			const previous = sent.get(item.type);
			if (previous?.text === text && time - previous.at < RESEND_AFTER_MS) continue;
			sent.set(item.type, { text, at: time });
			runtime.sendMessage({ report: item });
		}
	}

	runtime.onMessage.addListener((message) => {
		if (!context.top) return;
		if (message?.type === 'do') {
			performAction(doc, message.action);
		} else if (message?.type === 'openConversation' && typeof message.id === 'string') {
			openConversation(doc, message.id);
		}
	});
	report();
	schedule(report, POLL_INTERVAL_MS);
}
