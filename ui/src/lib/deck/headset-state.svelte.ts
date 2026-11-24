import type {
	AmbientSound,
	EqualizerChange,
	HeadsetInfo,
	HeadsetState as HeadsetStatus,
	ServerMessage
} from '#lib/generated/index.js';
import type { DeckConnection } from './deck-state.svelte.js';

/**
 * What the UI knows about the headphones: the connected ones and, for the one it follows,
 * the link to its settings channel and what it said. The link is opened by following them,
 * so they are only followed while the page that shows them is open ({@link enter}, {@link leave}).
 */
export class HeadsetState {
	headsets: HeadsetInfo[] = $state([]);
	/** What the headphones followed said; absent until the manager answered. */
	status: HeadsetStatus | undefined = $state();

	readonly #connection: DeckConnection;
	#watching = false;
	#address: string | undefined = $state();

	/** The headphones followed, if still connected; before they are followed, the first ones. */
	readonly headset: HeadsetInfo | undefined = $derived(
		this.headsets.find((candidate) => candidate.address === this.#address) ?? this.headsets[0]
	);

	constructor(connection: DeckConnection) {
		this.#connection = connection;
	}

	/** The page is shown: follows the headphones, which opens the link to them. */
	enter(): void {
		this.#watching = true;
		this.#follow();
	}

	/** The page is left: the link stays as it is, nothing is followed. */
	leave(): void {
		this.#watching = false;
		this.#forget();
	}

	/** Opens the link again, after a cut or a release. */
	resume(): boolean {
		return this.#send((address) => ({ type: 'resumeHeadset', address }));
	}

	/** Closes the link on purpose, for another application to use the headphones. */
	release(): boolean {
		return this.#send((address) => ({ type: 'releaseHeadset', address }));
	}

	/** Asks for an ambient sound; the headphones confirm it. */
	setAmbient(ambient: AmbientSound): boolean {
		return this.#send((address) => ({ type: 'setHeadsetAmbient', address, ambient }));
	}

	/** Asks for an equalizer change; the headphones confirm it. */
	setEqualizer(equalizer: EqualizerChange): boolean {
		return this.#send((address) => ({ type: 'setHeadsetEqualizer', address, equalizer }));
	}

	/** A new connection: what the headphones said may be out of date; the list that follows selects them again. */
	reconnected(): void {
		this.status = undefined;
	}

	/** Takes the headphones messages; the others are not its business. */
	handle(message: ServerMessage): void {
		if (message.type === 'headsets') {
			this.headsets = message.headsets;
			if (this.#watching) this.#follow();
		} else if (message.type === 'headsetState' && message.address === this.#address) {
			this.status = message.state;
		}
	}

	#send(build: (address: string) => Parameters<DeckConnection['send']>[0]): boolean {
		return this.#address !== undefined && this.#connection.send(build(this.#address));
	}

	/**
	 * Follows the current headphones when still connected, else the first. Sent again on every
	 * list: the manager ends the subscription when the headphones go away.
	 */
	#follow(): void {
		const target = [this.#address, this.headsets[0]?.address].find(
			(address) => address !== undefined && this.headsets.some((h) => h.address === address)
		);
		if (target === undefined) {
			this.#forget();
			return;
		}
		if (target !== this.#address) this.status = undefined;
		this.#address = target;
		this.#connection.send({ type: 'selectHeadset', address: target });
	}

	#forget(): void {
		this.#address = undefined;
		this.status = undefined;
	}
}
