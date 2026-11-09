<!--
@component
Touch strip of the device, drawn above the dials: each zone shows the icon of the module of the
dial below it. Decorative: the dials underneath carry the names.
-->
<script lang="ts">
	import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';

	interface Props {
		/** Instance of each dial (`null`: none assigned), as in the dial row. */
		dials: readonly (ModuleInstance | null)[];
		moduleOf: (kind: string) => CatalogModule | undefined;
	}

	let { dials, moduleOf }: Props = $props();
</script>

<ol
	aria-hidden="true"
	class="grid gap-3 rounded-2xl border border-device-line bg-device-strip py-3"
	style:grid-template-columns="repeat({dials.length}, minmax(0, 6rem))"
>
	{#each dials as instance, index (index)}
		{@const icon = instance ? moduleOf(instance.kind)?.icon : null}
		<li class="flex h-10 items-center justify-center">
			{#if icon}
				<img src={icon} alt="" class="size-10 rounded-lg object-contain" draggable="false" />
			{:else}
				<span class="h-1 w-6 rounded-full bg-device-line"></span>
			{/if}
		</li>
	{/each}
</ol>
