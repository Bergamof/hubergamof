<!--
@component
Vibrations of a haptic mouse: a monitor that draws and plays what the mouse feels, the strength (which also
silences everything when off) and, for each event a module can signal, whether it is played, how, and a
button to try it on the mouse. An event without a choice of its own plays the way its module made it.
Every change is applied at once; changing a waveform plays it.
-->
<script lang="ts">
	import {
		HAPTIC_GAINS,
		HAPTIC_LEVELS,
		HAPTIC_PATTERNS,
		HAPTIC_WAVEFORMS
	} from '#lib/deck/mouse-labels.js';
	import { hapticKey } from '#lib/deck/mouse-edits.js';
	import type {
		CatalogModule,
		HapticEventDescriptor,
		HapticEventSetting,
		HapticLevel,
		HapticSettings,
		HapticWaveform
	} from '#lib/generated/index.js';
	import SegmentedControl from './SegmentedControl.svelte';
	import Switch from './Switch.svelte';

	interface Props {
		haptics: HapticSettings;
		catalog: readonly CatalogModule[];
		/** Saves the strength; resolves to whether the manager took it. */
		onlevel: (level: HapticLevel) => Promise<boolean>;
		/** `null`: back to what the module chose. */
		onevent: (key: string, setting: HapticEventSetting | null) => void;
		/** Has the mouse play a waveform. */
		onplay: (waveform: HapticWaveform) => void;
	}

	let { haptics, catalog, onlevel, onevent, onplay }: Props = $props();

	const uid = $props.id();
	/** Time between two steps of a waveform, in milliseconds. */
	const STEP_MS = 60;
	/** How long the mouse of the monitor keeps shaking after the last step. */
	const SETTLE_MS = 800;
	/** Share of the width the mouse of the monitor moves by at the strongest. */
	const SHAKE_PIXELS = 5;
	const IDLE_MESSAGE = 'Choisissez une onde pour la sentir.';
	const OFF_MESSAGE = 'Retour haptique désactivé : rien n’est joué.';

	const modules = $derived(catalog.filter((module) => module.hapticEvents.length > 0));
	const labelOf = (waveform: HapticWaveform) =>
		HAPTIC_WAVEFORMS.find((candidate) => candidate.value === waveform)?.label ?? waveform;

	/** The waveform the monitor shows, how many of its steps have been played, and the shake of the mouse. */
	let shown: HapticWaveform | undefined = $state();
	let litSteps = $state(0);
	let shake = $state(0);
	let message = $state(IDLE_MESSAGE);
	let timers: ReturnType<typeof setTimeout>[] = [];

	const pattern = $derived(shown ? HAPTIC_PATTERNS[shown] : '0'.repeat(12));

	function stopTimers() {
		timers.forEach(clearTimeout);
		timers = [];
	}

	$effect(() => stopTimers);

	/** Draws and plays `waveform` on the monitor, at the strength `level`. */
	function animate(waveform: HapticWaveform, level: HapticLevel) {
		stopTimers();
		shown = waveform;
		litSteps = 0;
		shake = 0;
		if (level === 'off') {
			message = OFF_MESSAGE;
			return;
		}
		message = `Joué sur la souris : ${labelOf(waveform)}`;
		const gain = HAPTIC_GAINS[level];
		[...HAPTIC_PATTERNS[waveform]].forEach((digit, step) => {
			const strength = Number(digit);
			timers.push(
				setTimeout(() => {
					litSteps = step + 1;
					shake = strength > 0 ? (step % 2 ? 1 : -1) * strength * gain * (SHAKE_PIXELS / 9) : 0;
				}, step * STEP_MS)
			);
		});
		timers.push(setTimeout(() => (shake = 0), SETTLE_MS));
	}

	/** Has the mouse play `waveform` and shows it. */
	function play(waveform: HapticWaveform) {
		onplay(waveform);
		animate(waveform, haptics.level);
	}

	async function chooseLevel(level: HapticLevel) {
		const saved = await onlevel(level);
		// The mouse plays at the strength in force: only once it is the new one.
		if (saved) {
			onplay('sharpStateChange');
			animate('sharpStateChange', level);
		}
	}

	/** How the event is played now: the choice made here, else the module's default. */
	function settingOf(module: CatalogModule, event: HapticEventDescriptor): HapticEventSetting {
		return (
			haptics.events[hapticKey(module.kind, event.id)] ?? {
				enabled: true,
				waveform: event.waveform
			}
		);
	}

	function change(
		module: CatalogModule,
		event: HapticEventDescriptor,
		change: Partial<HapticEventSetting>
	) {
		const next = { ...settingOf(module, event), ...change };
		const isDefault = next.enabled && next.waveform === event.waveform;
		onevent(hapticKey(module.kind, event.id), isDefault ? null : next);
	}

	function chooseWaveform(
		module: CatalogModule,
		event: HapticEventDescriptor,
		waveform: HapticWaveform
	) {
		change(module, event, { waveform });
		play(waveform);
	}

	/** How many of the events of the module are on. */
	function enabledCount(module: CatalogModule): number {
		return module.hapticEvents.filter((event) => settingOf(module, event).enabled).length;
	}

	function setModule(module: CatalogModule, enabled: boolean) {
		for (const event of module.hapticEvents) {
			if (settingOf(module, event).enabled !== enabled) change(module, event, { enabled });
		}
	}
</script>

<div class="flex flex-col gap-5">
	<div class="flex flex-wrap items-center gap-x-5 gap-y-3 rounded-2xl bg-sunken px-4 py-3.5">
		<svg viewBox="0 0 54 70" class="h-[70px] w-[54px] flex-none" aria-hidden="true">
			<g style="transform: translateX({shake}px); transition: transform 0.05s">
				<path
					d="M10 62C5 50 4 30 8 18C11 8 20 3 27 3C36 3 44 8 46 18C50 30 49 50 44 62C40 68 14 68 10 62Z"
					fill="#2c4549"
					stroke="#4b696d"
				/>
				<path d="M27 3V26" stroke="#4b696d" />
				<rect x="24" y="10" width="6" height="12" rx="3" fill="#142426" />
			</g>
		</svg>
		<div class="flex min-w-0 flex-col gap-1.5">
			<span class="flex h-[52px] items-end gap-[3px]" aria-hidden="true">
				{#each [...pattern] as digit, step (step)}
					<i
						class={[
							'w-[9px] rounded-sm',
							step < litSteps && digit !== '0' ? 'bg-accent' : 'bg-line'
						]}
						style="height: {Math.max(8, Number(digit) * 11)}%"
					></i>
				{/each}
			</span>
			<span class="font-mono text-xs text-muted" role="status">{message}</span>
		</div>
		<div class="ml-auto flex flex-col gap-1.5">
			<span class="text-sm font-semibold" id="{uid}-level">Intensité générale</span>
			<SegmentedControl
				label="Force des vibrations"
				options={HAPTIC_LEVELS}
				value={haptics.level}
				onchange={chooseLevel}
			/>
		</div>
	</div>

	<p class="text-sm text-muted">
		Chaque module propose ses événements. Changer l’onde la joue aussitôt sur la souris, et « Jouer
		» l’essaie à la force réglée. L’intensité est un réglage général de la souris ; « Aucune
		vibration » coupe tout sur cette souris, y compris le menu circulaire.
	</p>

	{#each modules as module (module.kind)}
		{@const count = enabledCount(module)}
		<section class="flex flex-col" aria-labelledby="{uid}-{module.kind}">
			<h3
				id="{uid}-{module.kind}"
				class="flex items-center gap-2 border-b-2 border-line pb-1.5 text-base font-semibold"
			>
				{#if module.icon}
					<img src={module.icon} alt="" class="size-5" />
				{/if}
				{module.name}
				<span class="ml-auto flex items-center gap-2 text-xs font-normal text-muted">
					Tout le module
					<Switch
						checked={count === module.hapticEvents.length}
						indeterminate={count > 0 && count < module.hapticEvents.length}
						label="Activer ou désactiver tous les retours de {module.name}"
						onchange={(enabled) => setModule(module, enabled)}
					/>
				</span>
			</h3>
			{#each module.hapticEvents as event (event.id)}
				{@const setting = settingOf(module, event)}
				<div
					class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3.5 gap-y-2 border-b border-line py-2.5 sm:grid-cols-[38px_minmax(0,1.4fr)_minmax(0,1fr)_70px_auto]"
				>
					<span class="col-start-2 row-start-1 sm:col-start-auto sm:row-start-auto sm:contents">
						<Switch
							checked={setting.enabled}
							label="Activer : {event.name}"
							onchange={(enabled) => change(module, event, { enabled })}
						/>
					</span>
					<span class={['sm:order-none', setting.enabled ? '' : 'text-muted']}>
						<span class="font-medium">{event.name}</span>
						<span class="block text-sm text-muted">{event.description}</span>
					</span>
					<label class="sr-only" for="{uid}-{module.kind}-{event.id}">
						Vibration de : {module.name}, {event.name}
					</label>
					<select
						id="{uid}-{module.kind}-{event.id}"
						class="field"
						value={setting.waveform}
						disabled={!setting.enabled}
						onchange={(e) => chooseWaveform(module, event, e.currentTarget.value as HapticWaveform)}
					>
						{#each HAPTIC_WAVEFORMS as waveform (waveform.value)}
							<option value={waveform.value}>{waveform.label}</option>
						{/each}
					</select>
					<span class="flex h-[34px] items-end gap-[3px]" aria-hidden="true">
						{#each [...HAPTIC_PATTERNS[setting.waveform]] as digit, step (step)}
							<i
								class="w-[5px] rounded-sm bg-line"
								style="height: {Math.max(8, Number(digit) * 11)}%"
							></i>
						{/each}
					</span>
					<button
						type="button"
						class="secondary-button"
						disabled={!setting.enabled}
						aria-label="Jouer : {event.name}"
						onclick={() => play(setting.waveform)}
					>
						Jouer
					</button>
				</div>
			{/each}
		</section>
	{:else}
		<p class="text-sm text-muted">Aucun module ne signale d’événement par vibration.</p>
	{/each}
</div>
