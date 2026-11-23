<!--
@component
Drawings of a mouse (seen from above, and from the thumb side) with a numbered marker on each control. The drawings are
SVG files with no background, shown as images (`mouse-art.ts` places the zones and markers), the same in both themes. Pressing
a marker, or the control itself on the drawing, chooses it. The markers are what assistive technologies and the
keyboard use; the zones on the drawing are only a shortcut for the pointer.
-->
<script lang="ts">
	import type { MouseControlId, MouseModel } from '#lib/generated/index.js';
	import { MOUSE_ART, markerScale, type HitShape, type MouseView } from './mouse-art.js';

	interface Props {
		model: MouseModel;
		/** Name of the mouse, for the description of the drawings. */
		name: string;
		/** The control chosen. */
		selected: MouseControlId | undefined;
		/** Controls that cannot be given another action: their marker is dashed. */
		locked: readonly MouseControlId[];
		/** What each control does now, read with its marker. */
		summaries: Partial<Record<MouseControlId, string>>;
		onselect: (id: MouseControlId) => void;
	}

	let { model, name, selected, locked, summaries, onselect }: Props = $props();

	/** The elements are made by snippets, away from their `<svg>`: they need their namespace told. */
	const SVG_NAMESPACE = 'http://www.w3.org/2000/svg';

	const art = $derived(MOUSE_ART[model]);

	function spotsOf(view: MouseView) {
		return art.spots.filter((spot) => spot.view === view.id);
	}

	function markerKey(event: KeyboardEvent, id: MouseControlId) {
		if (event.key !== 'Enter' && event.key !== ' ') return;
		event.preventDefault();
		onselect(id);
	}
</script>

{#snippet zone(hit: HitShape)}
	<!-- The markers are the keyboard and screen reader way to the same choice. -->
	<svelte:element
		this={hit.tag}
		xmlns={SVG_NAMESPACE}
		{...hit.attrs}
		role="presentation"
		class={['part', selected === hit.id && 'sel', locked.includes(hit.id) && 'locked']}
		data-id={hit.id}
		onclick={() => onselect(hit.id)}
	/>
{/snippet}

<div class="flex flex-wrap items-end justify-center gap-x-5 gap-y-3">
	{#each art.views as view (view.id)}
		<figure class="m-0 flex min-w-0 flex-col items-center gap-1">
			<svg
				class="h-auto max-w-full overflow-visible drop-shadow-[0_10px_14px_rgb(0_0_0/0.28)]"
				style="width:{view.width}px"
				viewBox="0 0 {view.size.width} {view.size.height}"
				role="group"
				aria-label="{name}, {view.caption.toLowerCase()}"
			>
				<image href={view.src} width={view.size.width} height={view.size.height} />
				{#each view.hits as hit (hit.id)}
					{@render zone(hit)}
				{/each}
				{#each spotsOf(view) as spot (spot.id)}
					<g
						class={['mk', locked.includes(spot.id) && 'locked', selected === spot.id && 'sel']}
						data-id={spot.id}
						role="button"
						tabindex="0"
						aria-label="{spot.number}, {spot.label} : {summaries[spot.id] ?? ''}"
						aria-current={selected === spot.id ? 'true' : undefined}
						transform="translate({spot.x} {spot.y}) scale({markerScale(view)})"
						onclick={() => onselect(spot.id)}
						onkeydown={(event) => markerKey(event, spot.id)}
					>
						<circle class="halo" r="16" />
						<circle r="11" />
						<text>{spot.number}</text>
					</g>
				{/each}
			</svg>
			<figcaption class="font-mono text-xs tracking-wide text-muted uppercase">
				{view.caption}
			</figcaption>
		</figure>
	{/each}
</div>

<style>
	.part {
		fill: transparent;
		stroke: none;
		cursor: pointer;
		transition: fill 0.12s;
	}
	.part:hover {
		fill: color-mix(in srgb, var(--accent) 32%, transparent);
	}
	.part.sel {
		fill: color-mix(in srgb, var(--accent) 40%, transparent);
		stroke: var(--accent);
		stroke-width: 4;
	}
	.part.locked {
		cursor: default;
	}
	.mk {
		cursor: pointer;
	}
	.mk circle {
		fill: var(--accent);
		stroke: #111;
		stroke-width: 2;
	}
	.mk text {
		font: 600 11px var(--font-mono);
		fill: var(--on-accent);
		text-anchor: middle;
		dominant-baseline: central;
		pointer-events: none;
	}
	.mk.locked circle {
		fill: var(--surface);
		stroke: var(--muted);
		stroke-dasharray: 3 2;
	}
	.mk.locked text {
		fill: var(--muted);
	}
	.mk .halo {
		fill: none;
		stroke: var(--accent);
		stroke-width: 2.5;
		stroke-dasharray: none;
		display: none;
	}
	.mk.sel .halo {
		display: block;
	}
	.mk:focus-visible {
		outline: none;
	}
	.mk:focus-visible .halo {
		display: block;
		stroke: var(--ring);
	}
</style>
