import type {
	CatalogModule,
	ClientMessage,
	DeviceConfig,
	DeviceInfo,
	ServerMessage,
	UpdateSettings,
	UpdateStatus
} from '#lib/generated/index.js';
import type { ConfigEdit } from './config-edits.js';
import {
	CONNECTION_LOST,
	EditQueue,
	type EditOptions,
	type EditOutcome
} from './edit-queue.svelte.js';
import { browserDevicePreference, type DevicePreference } from './device-preference.js';
import { ApplicationState } from './application-state.svelte.js';
import { GitState } from './git-state.svelte.js';
import { MachineState } from './machine-state.svelte.js';
import { SshState } from './ssh-state.svelte.js';
import { HeadsetState } from './headset-state.svelte.js';
import { MouseState } from './mouse-state.svelte.js';
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

export type { EditOptions, EditOutcome };
export { UPDATE_TIMEOUT_MS } from './edit-queue.svelte.js';

const DEVICE_GONE: Problem = {
	text: 'Modification non enregistrée : l’appareil n’est plus connecté.',
	detail: null
};

/**
 * What the UI knows about the manager: devices, module catalog and the configuration of
 * the selected device, edited through {@link DeckState.edit} (see {@link EditQueue} for
 * how updates are sent).
 */
export class DeckState {
	devices: DeviceInfo[] = $state([]);
	catalog: CatalogModule[] = $state([]);
	selectedSerial: string | undefined = $state();
	notices: Notice[] = $state([]);
	/** Version of the running manager, known once it has greeted us. */
	managerVersion: string | undefined = $state();
	/** How updates are handled; unknown until the manager has said. */
	updateSettings: UpdateSettings | undefined = $state();
	/** Outcome of the manager's check for a newer release. */
	updateStatus: UpdateStatus = $state({ type: 'unknown' });
	/** The connected mice, their configuration and the radial menu. */
	readonly mice: MouseState;
	/** The modules that live in the application, and whether the computer is locked. */
	readonly application: ApplicationState;
	/** The connected headphones and what they said. */
	readonly headsets: HeadsetState;
	/** The shell aliases of the machine, managed and found. */
	readonly machine: MachineState;
	/** The Git configuration of the machine, managed and found. */
	readonly git: GitState;
	/** The SSH hosts of the machine, managed and found. */
	readonly ssh: SshState;

	readonly #edits = new EditQueue<DeviceConfig>({
		send: (config) =>
			this.#connection.send({ type: 'updateConfig', serial: config.serial, config }),
		report: (problem) => this.report(problem),
		gone: DEVICE_GONE
	});
	readonly #connection: DeckConnection;
	readonly #preference: DevicePreference;
	#nextNoticeId = 1;

	/** Selected device, if still connected. */
	readonly device: DeviceInfo | undefined = $derived(
		this.devices.find((device) => device.serial === this.selectedSerial)
	);

	/** Configuration as displayed: the saved one with the pending edits applied. */
	get config(): DeviceConfig | undefined {
		return this.#edits.config;
	}

	constructor(connection: DeckConnection, preference: DevicePreference = browserDevicePreference) {
		this.#connection = connection;
		this.#preference = preference;
		this.mice = new MouseState(connection, (problem) => this.report(problem));
		this.application = new ApplicationState(connection);
		this.headsets = new HeadsetState(connection);
		this.machine = new MachineState(connection, (problem) => this.report(problem));
		this.git = new GitState(connection, (problem) => this.report(problem));
		this.ssh = new SshState(connection, (problem) => this.report(problem));
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
	edit(change: ConfigEdit, options: EditOptions = {}): Promise<EditOutcome> {
		return this.#edits.edit(change, options);
	}

	/** Asks the manager to look for a newer release now; the answer arrives as `updateStatus`. */
	checkForUpdates(): void {
		this.#connection.send({ type: 'checkForUpdates' });
	}

	/** Asks the manager to install the newer release it found. */
	installUpdate(): void {
		this.#connection.send({ type: 'installUpdate' });
	}

	/** Chooses whether the manager installs a newer release as soon as it finds one. */
	setAutoInstall(enabled: boolean): void {
		this.#connection.send({ type: 'setAutoInstall', enabled });
	}

	dismiss(id: number): void {
		this.notices = this.notices.filter((notice) => notice.id !== id);
	}

	#handle(message: ServerMessage): void {
		switch (message.type) {
			case 'hello':
				// New connection: whatever was pending died with the previous one.
				this.#edits.dropPending(CONNECTION_LOST);
				this.mice.dropPending(CONNECTION_LOST);
				this.application.dropPending(CONNECTION_LOST);
				this.headsets.reconnected();
				for (const state of [this.machine, this.git, this.ssh]) {
					state.dropPending(CONNECTION_LOST);
					state.reconnected();
				}
				this.managerVersion = message.version;
				break;
			case 'devices':
				this.devices = message.devices;
				this.#followPreferredDevice();
				break;
			case 'moduleCatalog':
				this.catalog = message.modules;
				break;
			case 'config':
				if (message.serial === this.selectedSerial) this.#edits.receive(message.config);
				break;
			case 'error':
				this.#receiveError(problemOf(message.code, message.message));
				break;
			case 'updateSettings':
				this.updateSettings = message.settings;
				break;
			case 'updateStatus':
				this.updateStatus = message.status;
				break;
			case 'mice':
			case 'mouseConfig':
			case 'mouseStatus':
			case 'radialMenu':
				this.mice.handle(message);
				break;
			case 'machine':
				this.machine.handle(message);
				break;
			case 'git':
				this.git.handle(message);
				break;
			case 'ssh':
				this.ssh.handle(message);
				break;
			case 'headsets':
			case 'headsetState':
				this.headsets.handle(message);
				break;
			case 'applicationModules':
			case 'locked':
				this.application.handle(message);
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
		this.#edits.forget();
	}

	/**
	 * Errors are not tied to requests: one arriving while an update is in flight answers it
	 * (of the deck, the mice, the machine, Git or SSH alike), else it is only shown.
	 */
	#receiveError(problem: Problem): void {
		const waiting = [this.#edits, this.mice, this.machine, this.git, this.ssh].filter(
			(edits) => edits.pending
		);
		if (waiting.length === 0) {
			if (this.application.pending) this.application.dropPending(problem);
			else this.report(problem);
		} else if (waiting.length === 1) {
			waiting[0].dropPending(problem);
		} else {
			// Shown once, whichever of them wanted it.
			for (const edits of waiting) edits.dropPending(problem, { silent: true });
			this.report(problem);
		}
	}

	/** Shows `problem` until dismissed: a failure, or a warning of the editor. */
	report(problem: Problem): void {
		this.notices = [...this.notices, { id: this.#nextNoticeId, ...problem }];
		this.#nextNoticeId += 1;
	}
}
