<!--
@component
The radial menu, shared by all the mice: how it looks, which button opens it on the selected mouse, and what it
holds. Every change is sent at once; the line under the title says whether the manager has saved it.
-->
<script lang="ts">
	import type { DeckState } from '#lib/deck/deck-state.svelte.js';
	import { CONTROL_LABELS } from '#lib/deck/mouse-labels.js';
	import type { RadialEdit } from '#lib/deck/mouse-edits.js';
	import { SaveTracker } from '#lib/deck/save-tracker.svelte.js';
	import type { MouseConfig, MouseControlId, MouseInfo } from '#lib/generated/index.js';
	import RadialMenuEditor from './RadialMenuEditor.svelte';
	import RadialPreview from './RadialPreview.svelte';
	import SaveStatusLine from './SaveStatusLine.svelte';

	interface Props {
		deck: DeckState;
		/** The selected mouse, when there is one: the button that opens the menu is its own. */
		mouse: MouseInfo | undefined;
		config: MouseConfig | undefined;
	}

	let { deck, mouse, config }: Props = $props();

	const uid = $props.id();
	const saves = new SaveTracker();
	const menu = $derived(deck.mice.radialMenu);

	/** The control of this mouse that opens the menu, if one has been given that action. */
	const trigger = $derived(
		(Object.entries(config?.bindings ?? {}) as [MouseControlId, { kind: string }][]).find(
			([, action]) => action.kind === 'radialMenu'
		)?.[0]
	);

	function edit(change: RadialEdit) {
		void saves.track(deck.mice.editRadialMenu(change));
	}
</script>

<div class="flex flex-col gap-8">
	<SaveStatusLine tracker={saves} />

	<section aria-labelledby="{uid}-title" class="flex flex-col gap-4 card">
		<h2 id="{uid}-title" class="font-display text-xl font-semibold">Menu circulaire</h2>
		<p class="text-sm text-muted">
			Le même pour toutes les souris. Il s’ouvre depuis le bouton auquel vous avez donné l’action «
			Menu circulaire ».
		</p>
		{#if !mouse}
			<p class="text-sm">
				Aucune souris connectée : le menu s’ouvrira depuis un bouton de souris MX.
			</p>
		{:else}
			<p class="text-sm" data-testid="menu-trigger">
				{#if trigger}
					Sur {mouse.displayName}, il s’ouvre avec : <strong>{CONTROL_LABELS[trigger]}</strong>.
				{:else}
					Sur {mouse.displayName}, aucun bouton ne l’ouvre : donnez l’action « Menu circulaire » à
					un bouton, dans l’onglet Touches.
				{/if}
			</p>
		{/if}
		{#if menu}
			<div class="grid gap-6 md:grid-cols-[18rem_1fr]">
				<RadialPreview {menu} catalog={deck.catalog} />
				<RadialMenuEditor {menu} catalog={deck.catalog} onedit={edit} />
			</div>
		{:else}
			<p class="text-muted">Chargement du menu…</p>
		{/if}
	</section>
</div>
