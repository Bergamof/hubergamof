import type {
	ApplicationModuleView,
	ClientMessage,
	JsonValue,
	ServerMessage
} from '#lib/generated/index.js';
import type { DeckConnection } from './deck-state.svelte.js';
import { UPDATE_TIMEOUT_MS, type EditOutcome } from './edit-queue.svelte.js';
import { sameJson } from './json.js';
import type { Problem } from './problems.js';

const NO_ANSWER: Problem = {
	text: 'Le gestionnaire n’a pas répondu : les réglages ne sont peut-être pas enregistrés.',
	detail: null
};

const SUPERSEDED: Problem = {
	text: 'Des réglages plus récents ont remplacé ceux-ci avant la réponse du gestionnaire.',
	detail: null
};

interface PendingSettings {
	kind: string;
	settings: JsonValue;
	settle: (outcome: EditOutcome) => void;
	timer: ReturnType<typeof setTimeout>;
}

/**
 * What the UI knows about the modules that live in the application (the pomodoro): what
 * each one shows and its settings, and whether the computer is locked, which makes the
 * manager refuse everything the UI does.
 */
export class ApplicationState {
	modules: ApplicationModuleView[] = $state([]);
	/** Whether the computer is locked: the interface is not to be used then. */
	locked = $state(false);

	readonly #connection: DeckConnection;
	#pending: PendingSettings | undefined;

	constructor(connection: DeckConnection) {
		this.#connection = connection;
	}

	/** Whether settings were sent and the manager has not answered yet. */
	get pending(): boolean {
		return this.#pending !== undefined;
	}

	/** What the module of `kind` shows, if the application holds it. */
	viewOf(kind: string): ApplicationModuleView | undefined {
		return this.modules.find((module) => module.kind === kind);
	}

	handle(message: ServerMessage): void {
		if (message.type === 'locked') {
			this.locked = message.locked;
		} else if (message.type === 'applicationModules') {
			this.modules = message.modules;
			this.#settleIfTaken();
		}
	}

	/** Runs an action of the module (one of those its catalog entry offers). */
	run(kind: string, action: string): boolean {
		return this.#connection.send({ type: 'moduleAction', module: kind, action });
	}

	/**
	 * Saves the settings of the module; resolves once the manager shows them as the module's
	 * (it always says so after taking them), or when they were refused or not answered.
	 */
	saveSettings(kind: string, settings: JsonValue): Promise<EditOutcome> {
		this.dropPending(SUPERSEDED);
		return new Promise((settle) => {
			const message: ClientMessage = {
				type: 'updateModuleSettings',
				module: kind,
				settings
			};
			if (!this.#connection.send(message)) {
				settle({ saved: false, problem: NO_ANSWER });
				return;
			}
			const timer = setTimeout(() => this.dropPending(NO_ANSWER), UPDATE_TIMEOUT_MS);
			this.#pending = { kind, settings, settle, timer };
		});
	}

	/** Ends the settings waiting for an answer as refused for `problem`. */
	dropPending(problem: Problem): void {
		const pending = this.#pending;
		if (!pending) return;
		this.#pending = undefined;
		clearTimeout(pending.timer);
		pending.settle({ saved: false, problem });
	}

	#settleIfTaken(): void {
		const pending = this.#pending;
		if (!pending) return;
		const shown = this.viewOf(pending.kind);
		if (!shown || !sameJson(shown.settings, pending.settings)) return;
		this.#pending = undefined;
		clearTimeout(pending.timer);
		pending.settle({ saved: true });
	}
}
