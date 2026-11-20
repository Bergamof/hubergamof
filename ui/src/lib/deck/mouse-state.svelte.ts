import type {
	MouseConfig,
	MouseInfo,
	RadialMenuConfig,
	ServerMessage
} from '#lib/generated/index.js';
import type { DeckConnection } from './deck-state.svelte.js';
import { EditQueue, type Edit, type EditOptions, type EditOutcome } from './edit-queue.svelte.js';
import type { Problem } from './problems.js';

const MOUSE_GONE: Problem = {
	text: 'Modification non enregistrée : la souris n’est plus connectée.',
	detail: null
};
const RADIAL_UNKNOWN: Problem = {
	text: 'Modification non enregistrée : le menu circulaire n’est pas encore chargé.',
	detail: null
};

/**
 * What the UI knows about the mice: the connected ones, the configuration of the selected
 * one and the radial menu, each edited like the deck configuration (see {@link EditQueue}).
 */
export class MouseState {
	mice: MouseInfo[] = $state([]);
	selectedSerial: string | undefined = $state();

	readonly #connection: DeckConnection;
	readonly #mouseEdits: EditQueue<MouseConfig>;
	readonly #radialEdits: EditQueue<RadialMenuConfig>;

	/** Selected mouse, if still connected. */
	readonly mouse: MouseInfo | undefined = $derived(
		this.mice.find((mouse) => mouse.serial === this.selectedSerial)
	);

	constructor(connection: DeckConnection, report: (problem: Problem) => void) {
		this.#connection = connection;
		this.#mouseEdits = new EditQueue<MouseConfig>({
			send: (config) =>
				connection.send({ type: 'updateMouseConfig', serial: config.serial, config }),
			report,
			gone: MOUSE_GONE
		});
		this.#radialEdits = new EditQueue<RadialMenuConfig>({
			send: (config) => connection.send({ type: 'updateRadialMenu', config }),
			report,
			gone: RADIAL_UNKNOWN
		});
	}

	/** Configuration of the selected mouse with the pending edits applied. */
	get config(): MouseConfig | undefined {
		return this.#mouseEdits.config;
	}

	/** Radial menu with the pending edits applied. */
	get radialMenu(): RadialMenuConfig | undefined {
		return this.#radialEdits.config;
	}

	/** Whether an update of the mice is waiting for the manager's answer. */
	get pending(): boolean {
		return this.#mouseEdits.pending || this.#radialEdits.pending;
	}

	/** Follows another mouse. */
	select(serial: string): void {
		if (serial !== this.selectedSerial) this.#follow(serial);
	}

	/** Changes the configuration of the selected mouse; resolves once saved or dropped. */
	edit(change: Edit<MouseConfig>, options: EditOptions = {}): Promise<EditOutcome> {
		return this.#mouseEdits.edit(change, options);
	}

	/** Changes the radial menu; resolves once saved or dropped. */
	editRadialMenu(change: Edit<RadialMenuConfig>, options: EditOptions = {}): Promise<EditOutcome> {
		return this.#radialEdits.edit(change, options);
	}

	/** Drops what waits for an answer, failed with `problem`. */
	dropPending(problem: Problem): void {
		this.#mouseEdits.dropPending(problem);
		this.#radialEdits.dropPending(problem);
	}

	/** Takes the mouse messages; the others are not its business. */
	handle(message: ServerMessage): void {
		switch (message.type) {
			case 'mice':
				this.mice = message.mice;
				this.#followKnownMouse();
				break;
			case 'mouseConfig':
				if (message.serial === this.selectedSerial) {
					this.#mouseEdits.receive(message.config);
				}
				break;
			case 'radialMenu':
				this.#radialEdits.receive(message.config);
				break;
			default:
				break;
		}
	}

	/**
	 * Keeps the current mouse when still connected, else the first. Selection is sent again
	 * on every list: the manager ends a subscription when the mouse goes away.
	 */
	#followKnownMouse(): void {
		const serials = this.mice.map((mouse) => mouse.serial);
		const target = [this.selectedSerial, serials[0]].find(
			(serial) => serial !== undefined && serials.includes(serial)
		);
		if (target) this.#follow(target);
		else this.#forgetMouse();
	}

	#follow(serial: string): void {
		if (serial !== this.selectedSerial) {
			this.#forgetMouse();
			this.selectedSerial = serial;
		}
		this.#connection.send({ type: 'selectMouse', serial });
	}

	#forgetMouse(): void {
		this.selectedSerial = undefined;
		this.#mouseEdits.forget();
	}
}
