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

<div class="flex flex-col gap-4">
	<SaveStatusLine tracker={saves} />

	<div class="grid grid-cols-[19rem_minmax(0,1fr)] items-start gap-4">
		<!-- The preview is always there, and follows the page while the settings are edited. -->
		<section aria-labelledby="{uid}-preview" class="sticky top-28 card">
			<h2 id="{uid}-preview" class="mb-2 font-display text-lg font-semibold">Aperçu</h2>
			{#if menu}
				<RadialPreview {menu} catalog={deck.catalog} />
			{:else}
				<div
					role="status"
					class="grid aspect-square place-items-center rounded-full border border-dashed border-edge text-muted"
				>
					Chargement du menu…
				</div>
			{/if}
		</section>

		<section aria-labelledby="{uid}-title" class="flex flex-col gap-3 card">
			<h2 id="{uid}-title" class="font-display text-lg font-semibold">Réglages</h2>
			<p class="text-muted">
				Le même pour toutes les souris. Il s’ouvre depuis le bouton auquel vous avez donné l’action
				« Menu circulaire ».
			</p>
			{#if !mouse}
				<p>Aucune souris connectée : le menu s’ouvrira depuis un bouton de souris MX.</p>
			{:else}
				<p data-testid="menu-trigger">
					{#if trigger}
						Sur {mouse.displayName}, il s’ouvre avec : <strong>{CONTROL_LABELS[trigger]}</strong>.
					{:else}
						Sur {mouse.displayName}, aucun bouton ne l’ouvre : donnez l’action « Menu circulaire » à
						un bouton, dans l’onglet Touches.
					{/if}
				</p>
			{/if}
			{#if menu}
				<RadialMenuEditor {menu} catalog={deck.catalog} onedit={edit} />
			{/if}
		</section>
	</div>
</div>
