<!--
@component
Configuration of the selected mouse (buttons, pointer and wheels, vibrations on a haptic model) and of the
radial menu. Every change is sent at once; the line under the title says whether the manager has saved it.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import type { DeckState, EditOutcome } from '#lib/deck/deck-state.svelte.js';
	import {
		bindControl,
		setDpi,
		setHapticEvent,
		setHapticLevel,
		setThumbWheel,
		setWheel,
		type MouseEdit,
		type RadialEdit
	} from '#lib/deck/mouse-edits.js';
	import { CONTROL_LABELS } from '#lib/deck/mouse-labels.js';
	import type { MouseConfig, MouseInfo, RadialMenuConfig } from '#lib/generated/index.js';
	import ButtonBindingRow from './ButtonBindingRow.svelte';
	import HapticsPanel from './HapticsPanel.svelte';
	import PointerSettings from './PointerSettings.svelte';
	import RadialMenuEditor from './RadialMenuEditor.svelte';

	interface Props {
		deck: DeckState;
		mouse: MouseInfo;
		config: MouseConfig;
		/** `undefined` until the manager has sent it. */
		radialMenu: RadialMenuConfig | undefined;
	}

	let { deck, mouse, config, radialMenu }: Props = $props();

	const uid = $props.id();

	/** Where the last change stands: waiting for the manager, saved, or refused (the notice says why). */
	let status: 'idle' | 'saving' | 'saved' | 'refused' = $state('idle');

	const buttons = $derived(
		mouse.capabilities.controls.filter((control) => control.kind === 'button')
	);

	/** What the status region says, emptied first so that a repeated message is read again. */
	let spoken = $state('');

	async function track(outcome: Promise<EditOutcome>) {
		status = 'saving';
		const saved = (await outcome).saved;
		status = saved ? 'saved' : 'refused';
		spoken = '';
		await tick();
		spoken = STATUS_TEXT[status];
	}

	function edit(change: MouseEdit) {
		void track(deck.mice.edit(change));
	}

	function editRadial(change: RadialEdit) {
		void track(deck.mice.editRadialMenu(change));
	}

	const STATUS_TEXT = {
		idle: '',
		saving: 'Enregistrement…',
		saved: 'Modifications enregistrées.',
		refused: 'Dernière modification refusée.'
	} as const;
</script>

<div class="flex flex-col gap-8">
	<!-- Kept in view while the page scrolls: the changes are made far from the top. -->
	<div class="sticky top-0 z-10 -mx-1 bg-page px-1 py-1">
		<p class="min-h-6 text-sm text-muted" aria-hidden="true">{STATUS_TEXT[status]}</p>
		<p role="status" class="sr-only">{spoken}</p>
	</div>

	<section aria-labelledby="{uid}-buttons" class="flex flex-col gap-2 card">
		<h3 id="{uid}-buttons" class="text-lg font-semibold">Boutons</h3>
		<p class="text-sm text-muted">
			Un bouton sans action choisie ici garde son comportement d’origine.
		</p>
		<div class="flex flex-col divide-y divide-line">
			{#each buttons as control (control.id)}
				<ButtonBindingRow
					label={CONTROL_LABELS[control.id]}
					locked={control.locked}
					action={config.bindings[control.id]}
					catalog={deck.catalog}
					onchange={(action) => edit(bindControl(control.id, action))}
				/>
			{/each}
		</div>
	</section>

	<section aria-labelledby="{uid}-pointer" class="flex flex-col gap-4 card">
		<h3 id="{uid}-pointer" class="text-lg font-semibold">Pointeur et molettes</h3>
		<PointerSettings
			pointer={config.pointer}
			wheel={config.wheel}
			thumbWheel={config.thumbWheel}
			hasThumbWheel={mouse.capabilities.thumbWheel}
			onpointer={(dpi) => edit(setDpi(dpi))}
			onwheel={(change) => edit(setWheel(change))}
			onthumbwheel={(change) => edit(setThumbWheel(change))}
		/>
	</section>

	{#if mouse.capabilities.haptics}
		<section aria-labelledby="{uid}-haptics" class="flex flex-col gap-4 card">
			<h3 id="{uid}-haptics" class="text-lg font-semibold">Vibrations</h3>
			<HapticsPanel
				haptics={config.haptics}
				catalog={deck.catalog}
				onlevel={(level) => edit(setHapticLevel(level))}
				onevent={(key, setting) => edit(setHapticEvent(key, setting))}
			/>
		</section>
	{/if}

	<section aria-labelledby="{uid}-radial" class="flex flex-col gap-4 card">
		<h3 id="{uid}-radial" class="text-lg font-semibold">Menu circulaire</h3>
		<p class="text-sm text-muted">
			Le même pour toutes les souris. Il s’ouvre depuis le bouton auquel vous avez donné l’action «
			Menu circulaire ».
		</p>
		{#if radialMenu}
			<RadialMenuEditor menu={radialMenu} catalog={deck.catalog} onedit={editRadial} />
		{:else}
			<p class="text-muted">Chargement du menu…</p>
		{/if}
	</section>
</div>
