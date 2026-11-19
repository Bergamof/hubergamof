<!--
@component
Dials of the device, left to right: a knob drawing and the name of the function given to each
(what each dial reminds is drawn on the touch strip above). Dials can be swapped with the mouse
(the keyboard way is in the details).
-->
<script lang="ts">
	import type { DialSummary } from '#lib/deck/display.js';
	import DialKnob from './DialKnob.svelte';

	interface Props {
		dials: readonly DialSummary[];
		selected: number | undefined;
		onselect: (index: number) => void;
		onswap: (from: number, to: number) => void;
	}

	let { dials, selected, onselect, onswap }: Props = $props();

	const DRAG_TYPE = 'application/x-hubergamof-dial';
	let dropTarget: number | undefined = $state();

	function startDrag(event: DragEvent, index: number) {
		event.dataTransfer?.setData(DRAG_TYPE, String(index));
		if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
	}

	function dragOver(event: DragEvent, index: number) {
		if (!event.dataTransfer?.types.includes(DRAG_TYPE)) return;
		event.preventDefault();
		dropTarget = index;
	}

	function drop(event: DragEvent, index: number) {
		const dragged = event.dataTransfer?.getData(DRAG_TYPE);
		event.preventDefault();
		dropTarget = undefined;
		// An empty value is no dial: `Number('')` would read it as dial 0.
		if (!dragged) return;
		const from = Number(dragged);
		if (Number.isInteger(from) && from !== index) onswap(from, index);
	}
</script>

<ol class="grid gap-3" style:grid-template-columns="repeat({dials.length}, minmax(0, 8rem))">
	{#each dials as dial, index (index)}
		<!-- Draggable wrapper rather than button: Firefox does not drag buttons. -->
		<li
			draggable="true"
			class={[
				'cursor-grab rounded-2xl active:cursor-grabbing',
				dropTarget === index && 'outline-2 outline-offset-2 outline-device-ring outline-dashed'
			]}
			ondragstart={(event) => startDrag(event, index)}
			ondragover={(event) => dragOver(event, index)}
			ondragleave={() => (dropTarget = undefined)}
			ondragend={() => (dropTarget = undefined)}
			ondrop={(event) => drop(event, index)}
		>
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
					<span class="text-sm text-device-muted">indisponible</span>
				{/if}
			</button>
		</li>
	{/each}
</ol>
