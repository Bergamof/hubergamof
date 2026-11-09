<!--
@component
Dials of the device, left to right, each showing the module assigned to it.
-->
<script lang="ts">
	import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
	import { instanceName } from '#lib/deck/display.js';
	import ModuleTile from './ModuleTile.svelte';

	interface Props {
		/** Instance of each dial (`null`: none assigned). */
		dials: readonly (ModuleInstance | null)[];
		moduleOf: (kind: string) => CatalogModule | undefined;
		selected: number | undefined;
		onselect: (index: number) => void;
	}

	let { dials, moduleOf, selected, onselect }: Props = $props();
</script>

<ol class="grid gap-3" style:grid-template-columns="repeat({dials.length}, minmax(0, 6rem))">
	{#each dials as instance, index (index)}
		{@const module = instance ? moduleOf(instance.kind) : undefined}
		{@const name = instance ? instanceName(instance, module) : 'Aucun module'}
		<li>
			<button
				type="button"
				title={name}
				aria-current={selected === index ? 'true' : undefined}
				class="surface-button"
				onclick={() => onselect(index)}
			>
				{#if instance}
					<ModuleTile icon={module?.icon} {name} unavailable={!module} />
				{:else}
					<span
						aria-hidden="true"
						class="flex aspect-square w-full items-center justify-center rounded-full border-2 border-dashed border-device-line text-xs text-device-muted"
					>
						Libre
					</span>
				{/if}
				<span class="w-full truncate">
					<span class="sr-only">Molette {index + 1} :</span>
					{name}
				</span>
				{#if instance && !module}
					<span class="text-xs text-device-muted">indisponible</span>
				{/if}
			</button>
		</li>
	{/each}
</ol>
