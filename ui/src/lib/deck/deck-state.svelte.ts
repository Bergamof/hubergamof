import type {
	CatalogModule,
	ClientMessage,
	DeviceConfig,
	DeviceInfo,
	ServerMessage
} from '#lib/generated/index.js';
import type { ConfigEdit } from './config-edits.js';
import { browserDevicePreference, type DevicePreference } from './device-preference.js';
import { sameJson } from './json.js';
import { problemOf, type Problem } from './problems.js';

/** The part of `ManagerConnection` the deck state relies on (eases test doubles). */
export interface DeckConnection {
	send(message: ClientMessage): boolean;
	onMessage(listener: (message: ServerMessage) => void): () => void;
}

/** An error shown to the user until dismissed. */
export interface Notice extends Problem {
	id: number;
}

/** How an edit ended: saved by the manager, or dropped (and why). */
export type EditOutcome = { saved: true } | { saved: false; problem: Problem };

export interface EditOptions {
	/**
	 * Whether a failure is shown as a notice (default). Off for callers showing it
	 * themselves, next to what failed.
	 */
	notify?: boolean;
}

/** Longest wait for the manager to answer an update before giving it up. */
export const UPDATE_TIMEOUT_MS = 10_000;

const NOT_CONNECTED: Problem = {
	text: 'Modification non enregistrée : pas de connexion au gestionnaire.',
	detail: null
};
const CONNECTION_LOST: Problem = {
	text: 'Modification non enregistrée : la connexion au gestionnaire a été perdue.',
	detail: null
};
const NO_ANSWER: Problem = {
	text: 'Modification abandonnée : le gestionnaire n’a pas répondu.',
	detail: null
};
const DEVICE_GONE: Problem = {
	text: 'Modification non enregistrée : l’appareil n’est plus connecté.',
	detail: null
};

interface Waiter {
	settle: (outcome: EditOutcome) => void;
	notify: boolean;
}

interface QueuedEdit extends Waiter {
	change: ConfigEdit;
}

interface Update {
	/** Configuration sent, with the revision it replaces. */
	config: DeviceConfig;
	waiters: Waiter[];
	timer: ReturnType<typeof setTimeout>;
}

/** Whether `received` is the manager saving `sent`: same content, next revision. */
function isSaved(received: DeviceConfig, sent: DeviceConfig): boolean {
	return (
		received.revision === sent.revision + 1 &&
		sameJson({ ...received, revision: 0 }, { ...sent, revision: 0 })
	);
}

/**
 * What the UI knows about the manager: devices, module catalog and the configuration of
 * the selected device, edited through {@link DeckState.edit}.
 *
 * Updates are serialised: one is in flight at a time and the next edits wait for its
 * answer, then go together on top of the configuration the manager saved, so that quick
 * successive changes (brightness slider) never carry an outdated revision. Errors are not
 * tied to requests: any error arriving while an update is in flight answers it, and the
 * waiting edits are dropped with it. A configuration saved by another client meanwhile
 * does not answer it (the manager then refuses it as stale). {@link config} shows the
 * pending edits right away.
 */
export class DeckState {
	devices: DeviceInfo[] = $state([]);
	catalog: CatalogModule[] = $state([]);
	selectedSerial: string | undefined = $state();
	notices: Notice[] = $state([]);

	/** Last configuration of the selected device sent by the manager. */
	#saved: DeviceConfig | undefined = $state();
	#inFlight: Update | undefined = $state();
	#queued: QueuedEdit[] = $state([]);

	readonly #connection: DeckConnection;
	readonly #preference: DevicePreference;
	#nextNoticeId = 1;

	/** Selected device, if still connected. */
	readonly device: DeviceInfo | undefined = $derived(
		this.devices.find((device) => device.serial === this.selectedSerial)
	);

	/** Configuration as displayed: the saved one with the pending edits applied. */
	readonly config: DeviceConfig | undefined = $derived.by(() => {
		const base = this.#inFlight?.config ?? this.#saved;
		return base && this.#queued.reduce((config, queued) => queued.change(config), base);
	});

	constructor(connection: DeckConnection, preference: DevicePreference = browserDevicePreference) {
		this.#connection = connection;
		this.#preference = preference;
	}

	/** Starts following the manager messages; returns the function that stops it. */
	listen(): () => void {
		return this.#connection.onMessage((message) => this.#handle(message));
	}

	/** Follows another device. */
	select(serial: string): void {
		if (serial === this.selectedSerial) return;
		this.#preference.save(serial);
		this.#follow(serial);
	}

	/** Catalog entry of `kind`, if the manager provides it. */
	moduleOf(kind: string): CatalogModule | undefined {
		return this.catalog.find((module) => module.kind === kind);
	}

	/** Changes the configuration of the selected device; resolves once saved or dropped. */
	edit(change: ConfigEdit, { notify = true }: EditOptions = {}): Promise<EditOutcome> {
		return new Promise((settle) => {
			const waiter = { settle, notify };
			if (!this.#saved) {
				this.#fail([waiter], DEVICE_GONE);
			} else if (this.#inFlight) {
				this.#queued = [...this.#queued, { ...waiter, change }];
			} else {
				this.#send(change(this.#saved), [waiter]);
			}
		});
	}

	dismiss(id: number): void {
		this.notices = this.notices.filter((notice) => notice.id !== id);
	}

	#handle(message: ServerMessage): void {
		switch (message.type) {
			case 'hello':
				// New connection: whatever was pending died with the previous one.
				this.#dropPending(CONNECTION_LOST);
				break;
			case 'devices':
				this.devices = message.devices;
				this.#followPreferredDevice();
				break;
			case 'moduleCatalog':
				this.catalog = message.modules;
				break;
			case 'config':
				if (message.serial === this.selectedSerial) this.#receiveConfig(message.config);
				break;
			case 'error':
				this.#receiveError(problemOf(message.code, message.message));
				break;
			case 'liveState':
			case 'pong':
				break;
		}
	}

	/**
	 * Keeps the current device when still connected, else the remembered one, else the first.
	 * Selection is sent again on every list: the manager ends a subscription when the device
	 * goes away and expects the UI to select it again once back.
	 */
	#followPreferredDevice(): void {
		const serials = this.devices.map((device) => device.serial);
		const remembered = this.#preference.load();
		const target = [this.selectedSerial, remembered, serials[0]].find(
			(serial) => serial !== undefined && serial !== null && serials.includes(serial)
		);
		if (target) this.#follow(target);
		else this.#forgetDevice();
	}

	#follow(serial: string): void {
		if (serial !== this.selectedSerial) {
			this.#forgetDevice();
			this.selectedSerial = serial;
		}
		this.#connection.send({ type: 'selectDevice', serial, liveState: false });
	}

	#forgetDevice(): void {
		this.selectedSerial = undefined;
		this.#saved = undefined;
		this.#dropPending(DEVICE_GONE);
	}

	#receiveConfig(config: DeviceConfig): void {
		this.#saved = config;
		const update = this.#inFlight;
		if (!update || !isSaved(config, update.config)) return;
		clearTimeout(update.timer);
		this.#inFlight = undefined;
		for (const waiter of update.waiters) waiter.settle({ saved: true });
		this.#sendQueued(config);
	}

	#receiveError(problem: Problem): void {
		if (this.#inFlight) this.#dropPending(problem);
		else this.#notify(problem);
	}

	#sendQueued(base: DeviceConfig): void {
		const queued = this.#queued;
		if (queued.length === 0) return;
		this.#queued = [];
		this.#send(
			queued.reduce((config, edit) => edit.change(config), base),
			queued
		);
	}

	#send(config: DeviceConfig, waiters: Waiter[]): void {
		if (!this.#connection.send({ type: 'updateConfig', serial: config.serial, config })) {
			this.#fail(waiters, NOT_CONNECTED);
			return;
		}
		const timer = setTimeout(() => this.#dropPending(NO_ANSWER), UPDATE_TIMEOUT_MS);
		this.#inFlight = { config, waiters, timer };
	}

	#dropPending(problem: Problem): void {
		const waiters: Waiter[] = [...(this.#inFlight?.waiters ?? []), ...this.#queued];
		clearTimeout(this.#inFlight?.timer);
		this.#inFlight = undefined;
		this.#queued = [];
		this.#fail(waiters, problem);
	}

	/** Settles the waiters as failed, with one notice if any of them wants it. */
	#fail(waiters: readonly Waiter[], problem: Problem): void {
		for (const waiter of waiters) waiter.settle({ saved: false, problem });
		if (waiters.some((waiter) => waiter.notify)) this.#notify(problem);
	}

	#notify(problem: Problem): void {
		this.notices = [...this.notices, { id: this.#nextNoticeId, ...problem }];
		this.#nextNoticeId += 1;
	}
}
