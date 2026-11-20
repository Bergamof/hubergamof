<!--
@component
The radial menu of the mice, the same for all of them: which modules it shows, in which order, what a click
on each does, and how it opens. The manager only accepts between {@link MIN_SHOWN_ENTRIES} and
{@link MAX_SHOWN_ENTRIES} entries shown, so what would break that is not offered. Changes are applied at once.
-->
<script lang="ts">
	import {
		HOLD_MAX_MS,
		HOLD_MIN_MS,
		MAX_SHOWN_ENTRIES,
		MIN_SHOWN_ENTRIES,
		addEntry,
		moveEntry,
		removeEntry,
		setEntryEnabled,
		setEntryPrimary,
		setHoldMs,
		setRadialOptions,
		shownCount,
		type RadialEdit
	} from '#lib/deck/mouse-edits.js';
	import type { CatalogModule, RadialMenuConfig } from '#lib/generated/index.js';
	import IntegerField from './IntegerField.svelte';

	interface Props {
		menu: RadialMenuConfig;
		catalog: readonly CatalogModule[];
		onedit: (edit: RadialEdit) => void;
	}

	let { menu, catalog, onedit }: Props = $props();

	const uid = $props.id();
	const CLOSE = '';

	const shown = $derived(shownCount(menu));
	const full = $derived(shown >= MAX_SHOWN_ENTRIES);
	const minimal = $derived(shown <= MIN_SHOWN_ENTRIES);
	const unlisted = $derived(
		catalog.filter(
			(module) =>
				module.radialActions.length > 0 &&
				!menu.entries.some((entry) => entry.module === module.kind)
		)
	);
	let toAdd = $state('');
	const chosenToAdd = $derived(unlisted.find((module) => module.kind === toAdd) ?? unlisted[0]);

	function moduleOf(kind: string): CatalogModule | undefined {
		return catalog.find((module) => module.kind === kind);
	}
</script>

<div class="flex flex-col gap-5">
	<p class="text-sm text-muted" id="{uid}-count">
		Entre {MIN_SHOWN_ENTRIES} et {MAX_SHOWN_ENTRIES} entrées affichées ({shown} actuellement).
	</p>

	<ul class="flex flex-col divide-y divide-line" aria-describedby="{uid}-count">
		{#each menu.entries as entry, index (entry.module)}
			{@const module = moduleOf(entry.module)}
			{@const name = module?.name ?? entry.module}
			<li class="flex flex-wrap items-center gap-x-4 gap-y-2 py-3">
				<label class="flex w-56 items-center gap-2 font-medium">
					<input
						type="checkbox"
						class="size-5 accent-ring focus-ring"
						checked={entry.enabled}
						disabled={entry.enabled ? minimal : full}
						onchange={(event) => onedit(setEntryEnabled(entry.module, event.currentTarget.checked))}
					/>
					{#if module?.icon}
						<img src={module.icon} alt="" class="size-6" />
					{/if}
					{name}
				</label>
				<label class="sr-only" for="{uid}-primary-{entry.module}">Action du clic sur : {name}</label
				>
				<select
					id="{uid}-primary-{entry.module}"
					class="field w-auto"
					value={entry.primary.kind === 'action' ? entry.primary.action : CLOSE}
					onchange={(event) =>
						onedit(
							setEntryPrimary(
								entry.module,
								event.currentTarget.value === CLOSE
									? { kind: 'close' }
									: { kind: 'action', action: event.currentTarget.value }
							)
						)}
				>
					<option value={CLOSE}>Fermer le menu</option>
					{#each module?.radialActions ?? [] as action (action.id)}
						<option value={action.id}>{action.name}</option>
					{/each}
					{#if entry.primary.kind === 'action' && !module?.radialActions.some((action) => action.id === (entry.primary as { action: string }).action)}
						<option value={entry.primary.action}>{entry.primary.action} (absente)</option>
					{/if}
				</select>
				<span class="flex gap-2">
					<button
						type="button"
						class="secondary-button"
						disabled={index === 0}
						aria-label="Monter : {name}"
						onclick={() => onedit(moveEntry(entry.module, -1))}>↑</button
					>
					<button
						type="button"
						class="secondary-button"
						disabled={index === menu.entries.length - 1}
						aria-label="Descendre : {name}"
						onclick={() => onedit(moveEntry(entry.module, 1))}>↓</button
					>
					<button
						type="button"
						class="secondary-button"
						disabled={entry.enabled && minimal}
						aria-label="Retirer : {name}"
						onclick={() => onedit(removeEntry(entry.module))}>Retirer</button
					>
				</span>
			</li>
		{/each}
	</ul>

	{#if chosenToAdd}
		<div class="flex flex-wrap items-center gap-2">
			<label class="font-medium" for="{uid}-add">Ajouter un module</label>
			<select
				id="{uid}-add"
				class="field w-auto"
				value={chosenToAdd.kind}
				onchange={(event) => (toAdd = event.currentTarget.value)}
			>
				{#each unlisted as module (module.kind)}
					<option value={module.kind}>{module.name}</option>
				{/each}
			</select>
			<button
				type="button"
				class="secondary-button"
				disabled={full}
				onclick={() => onedit(addEntry(chosenToAdd.kind, chosenToAdd.radialActions[0].id))}
			>
				Ajouter
			</button>
		</div>
	{/if}

	<IntegerField
		label="Durée du maintien"
		value={menu.holdMs}
		min={HOLD_MIN_MS}
		max={HOLD_MAX_MS}
		unit="ms"
		hint="Un appui plus court que cette durée ouvre le menu jusqu’au prochain clic ; un appui plus long le garde ouvert tant que le bouton est tenu."
		error="Entrez un nombre entier de {HOLD_MIN_MS} à {HOLD_MAX_MS}."
		onchange={(holdMs) => onedit(setHoldMs(holdMs))}
	/>
	<label class="flex items-center gap-2">
		<input
			type="checkbox"
			class="size-5 accent-ring focus-ring"
			checked={menu.animate}
			onchange={(event) => onedit(setRadialOptions({ animate: event.currentTarget.checked }))}
		/>
		Animer l’ouverture du menu
	</label>
	<label class="flex items-center gap-2">
		<input
			type="checkbox"
			class="size-5 accent-ring focus-ring"
			checked={menu.haptics}
			onchange={(event) => onedit(setRadialOptions({ haptics: event.currentTarget.checked }))}
		/>
		Vibrations du menu (ouverture, survol, choix) sur les souris haptiques
	</label>
</div>
