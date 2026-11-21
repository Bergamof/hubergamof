<!--
@component
A slider with its value written next to it. Changes are sent while dragging, at most every
{@link SEND_PERIOD_MS}, and the last one when the slider is let go.
-->
<script lang="ts">
	import { throttle } from '#lib/deck/throttle.js';

	interface Props {
		/** Name read by assistive technologies. */
		label: string;
		value: number;
		min: number;
		max: number;
		step?: number;
		/** Written after the value (`dpi`, `ms`…), and read with it. */
		unit?: string;
		onchange: (value: number) => void;
	}

	let { label, value, min, max, step = 1, unit = '', onchange }: Props = $props();

	/** Longest delay between two updates sent while dragging. */
	const SEND_PERIOD_MS = 150;
	const send = throttle((next: number) => onchange(next), SEND_PERIOD_MS);
	const spoken = $derived(unit ? `${value} ${unit}` : String(value));

	$effect(() => () => send.cancel());
</script>

<input
	type="range"
	aria-label={label}
	{min}
	{max}
	{step}
	{value}
	aria-valuetext={spoken}
	class="h-6 min-w-36 flex-1 accent-ring focus-ring"
	oninput={(event) => send(event.currentTarget.valueAsNumber)}
	onchange={(event) => send.flush(event.currentTarget.valueAsNumber)}
/>
<!-- Already announced by the slider (aria-valuetext): not read again at every step. -->
<output aria-hidden="true" class="min-w-16 font-mono text-sm tabular-nums">{spoken}</output>
