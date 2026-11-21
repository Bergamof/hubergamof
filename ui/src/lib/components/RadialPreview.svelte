<!--
@component
The radial menu as it will look: the entries shown, in order, as the manager draws them (a ring cut into petals,
the first on the top, then clockwise, each with its icon and name). Choosing a petal lights it and tells what a
click on it does. The menu has the same colours in both themes, like the real one.
-->
<script lang="ts">
	import {
		ICON_SIZE,
		LABEL_GAP,
		LABEL_SIZE,
		MARGIN,
		RING,
		petalShapes
	} from '#lib/deck/radial-geometry.js';
	import type { CatalogModule, RadialMenuConfig } from '#lib/generated/index.js';

	interface Props {
		menu: RadialMenuConfig;
		catalog: readonly CatalogModule[];
	}

	let { menu, catalog }: Props = $props();

	/** Half the side of the drawing. */
	const HALF = RING.outer + MARGIN;
	/** Distance from the top of the icon (or the baseline of the name) to the middle of a petal. */
	const CONTENT_HEIGHT = ICON_SIZE + LABEL_GAP + LABEL_SIZE;

	const shown = $derived(menu.entries.filter((entry) => entry.enabled));
	const shapes = $derived(petalShapes(shown.length));
	let chosen: string | undefined = $state();
	const details = $derived(shown.find((entry) => entry.module === chosen));

	function moduleOf(kind: string): CatalogModule | undefined {
		return catalog.find((module) => module.kind === kind);
	}

	function nameOf(kind: string): string {
		return moduleOf(kind)?.name ?? kind;
	}

	function clickText(entry: RadialMenuConfig['entries'][number]): string {
		if (entry.primary.kind === 'close') return 'Un clic ferme le menu.';
		const id = entry.primary.action;
		const action = moduleOf(entry.module)?.radialActions.find((candidate) => candidate.id === id);
		return `Un clic : ${action?.name ?? id}.`;
	}

	function toggle(kind: string) {
		chosen = chosen === kind ? undefined : kind;
	}
</script>

<div class="flex flex-col items-center gap-3">
	<svg
		role="group"
		aria-label="Aperçu du menu"
		viewBox="{-HALF} {-HALF} {2 * HALF} {2 * HALF}"
		class="w-full max-w-80"
	>
		{#each shown as entry, index (entry.module)}
			{@const module = moduleOf(entry.module)}
			{@const { path, centre } = shapes[index]}
			{@const lit = chosen === entry.module}
			<g
				role="button"
				tabindex="0"
				aria-pressed={lit}
				aria-label={nameOf(entry.module)}
				class="petal"
				onclick={() => toggle(entry.module)}
				onkeydown={(event) => {
					if (event.key === 'Enter' || event.key === ' ') {
						event.preventDefault();
						toggle(entry.module);
					}
				}}
			>
				<path
					d={path}
					fill-rule="evenodd"
					fill={lit ? '#d9a42b' : '#1f332c'}
					fill-opacity={lit ? 0.95 : 0.9}
				/>
				{#if module?.icon}
					<image
						href={module.icon}
						x={centre.x - ICON_SIZE / 2}
						y={centre.y - CONTENT_HEIGHT / 2}
						width={ICON_SIZE}
						height={ICON_SIZE}
					/>
					<text
						x={centre.x}
						y={centre.y + CONTENT_HEIGHT / 2}
						text-anchor="middle"
						font-size={LABEL_SIZE}
						font-weight="600"
						fill="#fff"
					>
						{nameOf(entry.module)}
					</text>
				{:else}
					<text
						x={centre.x}
						y={centre.y + LABEL_SIZE / 3}
						text-anchor="middle"
						font-size={LABEL_SIZE}
						font-weight="600"
						fill="#fff"
					>
						{nameOf(entry.module)}
					</text>
				{/if}
			</g>
		{/each}
	</svg>
	<p class="min-h-6 text-sm text-muted" aria-live="polite">
		{#if details}
			<strong>{nameOf(details.module)}</strong> — {clickText(details)}
		{:else}
			Choisissez un pétale pour voir ce que fait un clic dessus.
		{/if}
	</p>
</div>

<style>
	.petal {
		cursor: pointer;
		outline: none;
	}
	.petal:focus-visible path {
		stroke: var(--color-ring);
		stroke-width: 3;
	}
	.petal text {
		pointer-events: none;
	}
</style>
