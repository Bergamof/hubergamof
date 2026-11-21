<!--
@component
The controls of a mouse: its drawings with a marker on each control, the list of the controls with what each
one does, and the editor of the one chosen. A locked control (left and right click) is only described.
Changes are sent at once; `null` gives a control back to the mouse.
-->
<script lang="ts">
	import { describeAction, ORIGINAL_LABELS } from '#lib/deck/mouse-labels.js';
	import type {
		ButtonAction,
		CatalogModule,
		MouseConfig,
		MouseControlId,
		MouseInfo
	} from '#lib/generated/index.js';
	import ButtonActionEditor from './ButtonActionEditor.svelte';
	import { MOUSE_ART } from './mouse-art.js';
	import MouseDiagram from './MouseDiagram.svelte';

	interface Props {
		mouse: MouseInfo;
		config: MouseConfig;
		catalog: readonly CatalogModule[];
		onbind: (control: MouseControlId, action: ButtonAction | null) => void;
	}

	let { mouse, config, catalog, onbind }: Props = $props();

	const uid = $props.id();

	/** The controls with a button to press (the thumb wheel only turns). */
	const spots = $derived(
		MOUSE_ART[mouse.model].spots.filter((spot) =>
			mouse.capabilities.controls.some((control) => control.id === spot.id)
		)
	);
	/** Controls that keep their role: the two clicks, and the wheel that only turns (set in its own tab). */
	const lockedIds = $derived(
		mouse.capabilities.controls
			.filter((control) => control.locked || control.kind === 'rotation')
			.map((control) => control.id)
	);
	const rotationIds = $derived(
		mouse.capabilities.controls
			.filter((control) => control.kind === 'rotation')
			.map((control) => control.id)
	);
	const summaries = $derived(
		Object.fromEntries(
			spots.map((spot) => [
				spot.id,
				lockedIds.includes(spot.id)
					? ORIGINAL_LABELS[spot.id]
					: describeAction(config.bindings[spot.id], ORIGINAL_LABELS[spot.id], catalog)
			])
		) as Partial<Record<MouseControlId, string>>
	);

	/** The control that opens the radial menu is the first one to show; else the first that can be changed. */
	const firstControl = $derived(
		spots.find((spot) => config.bindings[spot.id]?.kind === 'radialMenu')?.id ??
			spots.find((spot) => !lockedIds.includes(spot.id))?.id ??
			spots[0]?.id
	);
	let chosen: MouseControlId | undefined = $state();
	const current = $derived(spots.find((spot) => spot.id === chosen)?.id ?? firstControl);
	const spot = $derived(spots.find((candidate) => candidate.id === current));
	const isLocked = $derived(current !== undefined && lockedIds.includes(current));
</script>

<div class="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)]">
	<div>
		<MouseDiagram
			model={mouse.model}
			name={mouse.displayName}
			selected={current}
			locked={lockedIds}
			{summaries}
			onselect={(id) => (chosen = id)}
		/>
		<p class="mx-auto mt-3.5 text-center text-sm text-muted">
			Repère en pointillés : bouton verrouillé. Les repères sont placés d’après les photos du
			modèle.
		</p>
	</div>

	<div class="flex min-w-0 flex-col gap-4">
		<ul class="m-0 flex list-none flex-col overflow-hidden rounded-xl border border-line p-0">
			{#each spots as item (item.id)}
				<li class="border-b border-line last:border-b-0">
					<button
						type="button"
						class="grid w-full grid-cols-[26px_minmax(0,1fr)_minmax(0,1.1fr)] items-center gap-2.5 bg-surface px-3 py-2.5 text-left focus-ring hover:bg-sunken aria-current:bg-accent/20 aria-current:shadow-[inset_3px_0_0_var(--color-accent)]"
						aria-current={current === item.id ? 'true' : undefined}
						onclick={() => (chosen = item.id)}
					>
						<span
							class={[
								'grid size-[22px] place-items-center rounded-full font-mono text-xs font-semibold',
								lockedIds.includes(item.id)
									? 'border-[1.5px] border-dashed border-muted text-muted'
									: 'bg-accent text-on-accent'
							]}
						>
							{item.number}
						</span>
						<span>{item.label}</span>
						<span class="font-mono text-xs [overflow-wrap:anywhere] text-muted">
							{summaries[item.id]}
						</span>
					</button>
				</li>
			{/each}
		</ul>

		{#if spot}
			<section aria-labelledby="{uid}-control" class="flex flex-col gap-3 card">
				<h3 id="{uid}-control" class="flex items-center gap-2 text-base font-semibold">
					<span
						aria-hidden="true"
						class={[
							'grid size-[22px] place-items-center rounded-full font-mono text-xs',
							isLocked
								? 'border-[1.5px] border-dashed border-muted text-muted'
								: 'bg-accent text-on-accent'
						]}
					>
						{spot.number}
					</span>
					{spot.label}
				</h3>
				<p class="text-sm text-muted">{spot.where}</p>
				{#if isLocked}
					<p class="rounded-[10px] border border-line bg-accent/15 p-2.5 text-sm" role="note">
						{#if rotationIds.includes(spot.id)}
							Cette molette n’a pas d’action à choisir : sa vitesse et son sens se règlent dans
							l’onglet « Pointeur et molette ».
						{:else}
							Ce bouton est verrouillé : il garde toujours son action d’origine, pour ne jamais
							perdre l’usage de la souris.
						{/if}
					</p>
				{:else}
					<ButtonActionEditor
						action={config.bindings[spot.id]}
						{catalog}
						onchange={(action) => onbind(spot.id, action)}
					/>
				{/if}
			</section>
		{/if}
	</div>
</div>
