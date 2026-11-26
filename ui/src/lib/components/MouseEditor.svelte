<!--
@component
Configuration of the selected mouse (buttons, pointer and wheels, vibrations on a haptic model). Every change is sent at once; the line under the title says whether the manager has saved it.
-->
<script lang="ts">
	import type { DeckState, EditOutcome } from '#lib/deck/deck-state.svelte.js';
	import {
		bindControl,
		setDpi,
		setHapticEvent,
		setHapticLevel,
		setThumbWheel,
		setWheel,
		type MouseEdit
	} from '#lib/deck/mouse-edits.js';
	import { SaveTracker } from '#lib/deck/save-tracker.svelte.js';
	import { settingLabel } from '#lib/deck/mouse-labels.js';
	import type { MouseConfig, MouseInfo } from '#lib/generated/index.js';
	import BatteryBadge from './BatteryBadge.svelte';
	import HapticsPanel from './HapticsPanel.svelte';
	import MouseControls from './MouseControls.svelte';
	import PageHeader from './PageHeader.svelte';
	import PointerSettings from './PointerSettings.svelte';
	import SaveStatusLine from './SaveStatusLine.svelte';
	import Tabs, { panelId, tabId } from './Tabs.svelte';

	interface Props {
		deck: DeckState;
		mouse: MouseInfo;
		config: MouseConfig;
	}

	let { deck, mouse, config }: Props = $props();

	const uid = $props.id();

	const saves = new SaveTracker();

	type TabId = 'touches' | 'settings' | 'haptic';
	let wanted: TabId = $state('touches');
	const tabs = $derived([
		{ id: 'touches' as const, label: 'Touches' },
		{ id: 'settings' as const, label: 'Pointeur et molette' },
		...(mouse.capabilities.haptics ? [{ id: 'haptic' as const, label: 'Haptique' }] : [])
	]);
	/** The tab shown: the one asked for, unless this mouse has not got it (another mouse was chosen). */
	const tab = $derived(tabs.some((candidate) => candidate.id === wanted) ? wanted : 'touches');

	function edit(change: MouseEdit): Promise<EditOutcome> {
		const outcome = deck.mice.edit(change);
		void saves.track(outcome);
		return outcome;
	}
</script>

<div class="flex flex-col gap-4">
	<PageHeader title={mouse.displayName}>
		{#snippet chips()}
			<span class="chip">Connectée · Logi Bolt</span>
			{#if mouse.battery}
				<span class="chip"><BatteryBadge battery={mouse.battery} labelled /></span>
			{/if}
			{#if mouse.capabilities.haptics}
				<span class="chip">Retour haptique</span>
			{/if}
		{/snippet}
	</PageHeader>

	<SaveStatusLine tracker={saves} />

	{#if deck.mice.problems.length > 0}
		<div
			role="alert"
			class="rounded-xl border border-warning bg-warning-soft p-3 text-sm text-warning"
		>
			<p class="font-medium">
				Enregistré, mais la souris n’a pas accepté tous les réglages. Les autres sont appliqués.
			</p>
			<ul class="mt-1 list-disc pl-5">
				{#each deck.mice.problems as problem (problem.setting)}
					<li>{settingLabel(problem.setting)} : {problem.message}</li>
				{/each}
			</ul>
		</div>
	{/if}

	<Tabs
		label="Réglages de {mouse.displayName}"
		{tabs}
		selected={tab}
		prefix={uid}
		onselect={(id) => (wanted = id)}
	/>

	<div
		role="tabpanel"
		id={panelId(uid, tab)}
		aria-labelledby={tabId(uid, tab)}
		tabindex="0"
		class="focus-ring"
	>
		{#if tab === 'touches'}
			<section class="card" aria-label="Touches">
				<MouseControls
					{mouse}
					{config}
					catalog={deck.catalog}
					onbind={(control, action) => edit(bindControl(control, action))}
				/>
			</section>
		{:else if tab === 'settings'}
			<PointerSettings
				pointer={config.pointer}
				wheel={config.wheel}
				thumbWheel={config.thumbWheel}
				hasThumbWheel={mouse.capabilities.thumbWheel}
				onpointer={(dpi) => edit(setDpi(dpi))}
				onwheel={(change) => edit(setWheel(change))}
				onthumbwheel={(change) => edit(setThumbWheel(change))}
			/>
		{:else}
			<section class="max-w-3xl card" aria-label="Retour haptique">
				<HapticsPanel
					haptics={config.haptics}
					catalog={deck.catalog}
					onlevel={async (level) => (await edit(setHapticLevel(level))).saved}
					onevent={(key, setting) => void edit(setHapticEvent(key, setting))}
					onplay={(waveform) => void deck.mice.playHaptic(waveform)}
				/>
			</section>
		{/if}
	</div>
</div>
