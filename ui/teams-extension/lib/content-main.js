import { performAction, readReports } from './teams-dom.js';

/** Teams changes its page without events we could rely on: look again regularly. */
export const POLL_INTERVAL_MS = 2000;

/**
 * Reports what the page shows, whenever it changes, and carries out the
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
	/** Last report sent of each type, to send changes only. */
	const sent = new Map();

	function report() {
		for (const item of readReports(doc, hostname, Math.floor(now() / 1000), context)) {
			const text = JSON.stringify(item);
			if (sent.get(item.type) === text) continue;
			sent.set(item.type, text);
			runtime.sendMessage({ report: item });
		}
	}

	runtime.onMessage.addListener((message) => {
		if (message?.type === 'do' && context.top) {
			performAction(doc, message.action);
		}
	});
	report();
	schedule(report, POLL_INTERVAL_MS);
}
