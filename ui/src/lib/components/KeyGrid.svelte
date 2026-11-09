<!--
@component
Keys of the home page, in device order, as a grid of the device width. Selecting a key opens
its details; keys can also be reordered with the mouse (the keyboard way is in the details).
-->
<script lang="ts">
	import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
	import { instanceName } from '#lib/deck/display.js';
	import ModuleTile from './ModuleTile.svelte';

	interface Props {
		/** Instance of each key, in order (`undefined`: dangling reference). */
		keys: readonly (ModuleInstance | undefined)[];
		moduleOf: (kind: string) => CatalogModule | undefined;
		/** Image shown on the key of `instance`: its own, else the module icon. */
		imageOf: (instance: ModuleInstance, module: CatalogModule | undefined) => string | null;
		columns: number;
		/** Rows shown at once by the device: further rows are reached by scrolling. */
		visibleRows: number;
		selected: number | undefined;
		/** Id of the key button at `position`, `-1` for the add button (focus after a removal). */
		buttonId: (position: number) => string;
		onselect: (position: number) => void;
		onmove: (from: number, to: number) => void;
		onadd: () => void;
	}

	let {
		keys,
		moduleOf,
		imageOf,
		columns,
		visibleRows,
		selected,
		buttonId,
		onselect,
		onmove,
		onadd
	}: Props = $props();

	const DRAG_TYPE = 'application/x-hubergamof-key';
	const overflows = $derived(keys.length >= columns * visibleRows);
	let dropTarget: number | undefined = $state();

	function startDrag(event: DragEvent, position: number) {
		event.dataTransfer?.setData(DRAG_TYPE, String(position));
		if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
	}

	function dragOver(event: DragEvent, position: number) {
		if (!event.dataTransfer?.types.includes(DRAG_TYPE)) return;
		event.preventDefault();
		dropTarget = position;
	}

	function drop(event: DragEvent, position: number) {
		const from = Number(event.dataTransfer?.getData(DRAG_TYPE));
		event.preventDefault();
		dropTarget = undefined;
		if (Number.isInteger(from) && from !== position) onmove(from, position);
	}
</script>

<ol class="grid gap-3" style:grid-template-columns="repeat({columns}, minmax(0, 6rem))">
	{#each keys as instance, position (position)}
		{@const module = instance && moduleOf(instance.kind)}
		{@const name = instance ? instanceName(instance, module) : 'Module manquant'}
		<!-- Draggable wrapper rather than button: Firefox does not drag buttons. -->
		<li
			draggable="true"
			class={[
				'rounded-2xl',
				dropTarget === position && 'outline-2 outline-offset-2 outline-device-ring outline-dashed'
			]}
			ondragstart={(event) => startDrag(event, position)}
			ondragover={(event) => dragOver(event, position)}
			ondragleave={() => (dropTarget = undefined)}
			ondragend={() => (dropTarget = undefined)}
			ondrop={(event) => drop(event, position)}
		>
			<button
				id={buttonId(position)}
				type="button"
				title={name}
				aria-current={selected === position ? 'true' : undefined}
				class="surface-button"
				onclick={() => onselect(position)}
			>
				<ModuleTile
					icon={instance ? imageOf(instance, module) : null}
					{name}
					unavailable={!module}
				/>
				<span class="w-full truncate">
					<span class="sr-only">Touche {position + 1} :</span>
					{name}
				</span>
				{#if !module}
					<span class="text-xs text-device-muted">indisponible</span>
				{/if}
			</button>
		</li>
	{/each}
	<li>
		<button
			id={buttonId(-1)}
			type="button"
			class="flex aspect-square w-full flex-col items-center justify-center gap-1 rounded-2xl border-2 border-dashed border-device-line text-xs text-device-muted hover:bg-device-key hover:text-device-fg focus-visible:outline-3 focus-visible:outline-offset-2 focus-visible:outline-device-ring"
			onclick={onadd}
		>
			<span aria-hidden="true" class="text-2xl leading-none">+</span>
			Ajouter une touche
		</button>
	</li>
</ol>
{#if overflows}
	<p class="mt-3 text-sm text-device-muted">
		L’appareil affiche {visibleRows} ligne{visibleRows > 1 ? 's' : ''} de {columns} touches à la fois
		: les suivantes s’atteignent en faisant défiler avec la molette Navigation.
	</p>
{/if}
