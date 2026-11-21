<!--
@component
The radial menu as it will look: the entries shown, in order, around the pointer. Choosing a petal tells what a
click on it does. Positions come from {@link petalPosition}, clockwise from the top like the real menu.
-->
<script lang="ts">
	import type { CatalogModule, RadialMenuConfig } from '#lib/generated/index.js';

	interface Props {
		menu: RadialMenuConfig;
		catalog: readonly CatalogModule[];
	}

	let { menu, catalog }: Props = $props();

	/** Share of the preview's side that a petal's center is away from the middle. */
	const RADIUS = 0.34;
	const FULL_TURN = 2 * Math.PI;

	const shown = $derived(menu.entries.filter((entry) => entry.enabled));
	let chosen: string | undefined = $state();
	const details = $derived(shown.find((entry) => entry.module === chosen));

	/** Center of petal `index` of `count`, as percentages of the preview's width and height. */
	function petalPosition(index: number, count: number): { left: number; top: number } {
		const angle = (index / count) * FULL_TURN - Math.PI / 2;
		return {
			left: 50 + 100 * RADIUS * Math.cos(angle),
			top: 50 + 100 * RADIUS * Math.sin(angle)
		};
	}

	function moduleOf(kind: string): CatalogModule | undefined {
		return catalog.find((module) => module.kind === kind);
	}

	function clickText(entry: RadialMenuConfig['entries'][number]): string {
		if (entry.primary.kind === 'close') return 'Un clic ferme le menu.';
		const id = entry.primary.action;
		const action = moduleOf(entry.module)?.radialActions.find((candidate) => candidate.id === id);
		return `Un clic : ${action?.name ?? id}.`;
	}
</script>

<div class="flex flex-col items-center gap-3">
	<ul
		aria-label="Aperçu du menu"
		class="relative aspect-square w-full max-w-72 rounded-full border border-line bg-sunken"
	>
		{#each shown as entry, index (entry.module)}
			{@const module = moduleOf(entry.module)}
			{@const position = petalPosition(index, shown.length)}
			<li
				class="absolute -translate-x-1/2 -translate-y-1/2"
				style:left="{position.left}%"
				style:top="{position.top}%"
			>
				<button
					type="button"
					class={[
						'flex size-14 items-center justify-center rounded-full border focus-ring',
						chosen === entry.module ? 'bg-accent-soft border-accent' : 'border-line bg-surface'
					]}
					aria-pressed={chosen === entry.module}
					aria-label={module?.name ?? entry.module}
					onclick={() => (chosen = chosen === entry.module ? undefined : entry.module)}
				>
					{#if module?.icon}
						<img src={module.icon} alt="" class="size-8" />
					{:else}
						<span aria-hidden="true">{(module?.name ?? entry.module).slice(0, 1)}</span>
					{/if}
				</button>
			</li>
		{/each}
	</ul>
	<p class="min-h-6 text-sm text-muted" aria-live="polite">
		{#if details}
			<strong>{moduleOf(details.module)?.name ?? details.module}</strong> — {clickText(details)}
		{:else}
			Choisissez un pétale pour voir ce que fait un clic dessus.
		{/if}
	</p>
</div>
