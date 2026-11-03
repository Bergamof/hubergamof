<!--
@component
Brightness slider: changes are sent while dragging, at most every {@link SEND_PERIOD_MS}.
-->
<script lang="ts">
	import { MAX_BRIGHTNESS } from '#lib/deck/config-edits.js';
	import { throttle } from '#lib/deck/throttle.js';

	interface Props {
		brightness: number;
		onchange: (brightness: number) => void;
	}

	let { brightness, onchange }: Props = $props();

	/** Longest delay between two updates sent while dragging. */
	const SEND_PERIOD_MS = 150;
	const STEP = 5;

	const uid = $props.id();
	const send = throttle((value: number) => onchange(value), SEND_PERIOD_MS);

	$effect(() => () => send.cancel());
</script>

<div class="flex flex-wrap items-center gap-3">
	<label for={uid} class="font-medium">Luminosité</label>
	<input
		id={uid}
		type="range"
		min="0"
		max={MAX_BRIGHTNESS}
		step={STEP}
		value={brightness}
		aria-valuetext="{brightness} %"
		class="h-6 w-48 accent-sky-700 focus-ring"
		oninput={(event) => send(event.currentTarget.valueAsNumber)}
		onchange={(event) => send.flush(event.currentTarget.valueAsNumber)}
	/>
	<!-- Already announced by the slider (aria-valuetext): not read again at every step. -->
	<output for={uid} aria-hidden="true" class="w-12 tabular-nums">{brightness} %</output>
</div>
