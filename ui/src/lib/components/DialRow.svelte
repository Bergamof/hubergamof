<!--
@component
Dials of the device, left to right: a knob drawing and the name of the function given to each
(what each dial reminds is drawn on the touch strip above).
-->
<script lang="ts">
	import type { DialSummary } from '#lib/deck/display.js';
	import DialKnob from './DialKnob.svelte';

	interface Props {
		dials: readonly DialSummary[];
		selected: number | undefined;
		onselect: (index: number) => void;
	}

	let { dials, selected, onselect }: Props = $props();
</script>

<ol class="grid gap-3" style:grid-template-columns="repeat({dials.length}, minmax(0, 8rem))">
	{#each dials as dial, index (index)}
		<li>
			<button
				type="button"
				title={dial.name}
				aria-current={selected === index ? 'true' : undefined}
				class="surface-button"
				onclick={() => onselect(index)}
			>
				<DialKnob assigned={dial.assigned} />
				<span class="w-full truncate">
					<span class="sr-only">Molette {index + 1} :</span>
					{dial.name}
				</span>
				{#if dial.missing}
					<span class="text-xs text-device-muted">indisponible</span>
				{/if}
			</button>
		</li>
	{/each}
</ol>
