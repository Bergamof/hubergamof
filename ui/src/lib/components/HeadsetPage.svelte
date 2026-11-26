<!--
@component
The headphones: the link to their settings channel (they serve one client at a time, the Sony app or this
one), their battery, the ambient sound and the equalizer. The link opens when the page does and is never
retried after a cut: the person asks for it. Every change is asked of the headphones, which confirm it; the
controls show what they said.
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import type { DeckState } from '#lib/deck/deck-state.svelte.js';
	import {
		AMBIENT_MODES,
		EQUALIZER_BANDS,
		EQUALIZER_MAX,
		EQUALIZER_MIN,
		LINK_LABELS,
		MAX_AMBIENT_LEVEL,
		presetOptions
	} from '#lib/deck/headset-labels.js';
	import type { AmbientMode, EqualizerPreset } from '#lib/generated/index.js';
	import BatteryBadge from './BatteryBadge.svelte';
	import Icon from './Icon.svelte';
	import PageHeader from './PageHeader.svelte';
	import RangeControl from './RangeControl.svelte';
	import SegmentedControl from './SegmentedControl.svelte';
	import SelectField from './SelectField.svelte';
	import SettingRow from './SettingRow.svelte';
	import Switch from './Switch.svelte';

	interface Props {
		deck: DeckState;
	}

	let { deck }: Props = $props();

	const headsets = $derived(deck.headsets);
	const status = $derived(headsets.status);
	const link = $derived(status?.link ?? 'idle');
	const ambient = $derived(status?.ambient ?? undefined);
	const equalizer = $derived(status?.equalizer ?? undefined);
	const battery = $derived(
		status?.battery ?? (headsets.headset ? headsets.batteryOf(headsets.headset.address) : undefined)
	);
	const connected = $derived(link === 'connected');
	const modes = $derived(
		AMBIENT_MODES.filter((mode) => mode.value !== 'windReduction' || status?.windReduction)
	);

	// Opening the page follows the headphones: that reads and writes the state this page shows.
	$effect(() => {
		untrack(() => headsets.enter());
		return () => untrack(() => headsets.leave());
	});

	function onlink() {
		if (connected) headsets.release();
		else if (link === 'released' || link === 'cut') headsets.resume();
	}

	function setMode(mode: AmbientMode) {
		if (ambient) headsets.setAmbient({ ...ambient, mode });
	}

	function setBand(index: number, value: number) {
		if (!equalizer) return;
		const bands = [...equalizer.bands] as typeof equalizer.bands;
		bands[index] = value;
		headsets.setEqualizer({ type: 'manual', clearBass: equalizer.clearBass, bands });
	}

	function setPreset(preset: EqualizerPreset) {
		headsets.setEqualizer({ type: 'preset', preset });
	}
</script>

{#if !headsets.headset}
	<PageHeader title="Casque" />
	<p class="m-auto text-muted">Aucun casque connecté à cet ordinateur.</p>
{:else}
	<div class="flex flex-col gap-4">
		<PageHeader title={headsets.headset.name}>
			{#snippet chips()}
				<span class="chip" role={link === 'cut' ? 'alert' : 'status'}>{LINK_LABELS[link]}</span>
				{#if battery}
					<span class="chip"><BatteryBadge {battery} labelled /></span>
				{/if}
			{/snippet}
		</PageHeader>

		<section
			class="flex flex-wrap items-center gap-3 rounded-xl border border-warning bg-warning-soft p-3 text-sm"
			aria-label="Connexion"
		>
			<Icon name="info" class="size-4 shrink-0 text-warning" />
			<p class="flex-1">
				Un seul client de réglages à la fois : cette page ouvre la liaison et coupe l’application
				Sony du téléphone (pas l’audio).
			</p>
			<!-- One button whose action follows the link: the focus stays on it whatever the link does. -->
			<button
				type="button"
				class={connected ? 'secondary-button' : 'primary-button'}
				aria-disabled={link === 'idle' || link === 'connecting'}
				onclick={onlink}
			>
				{connected ? 'Libérer' : 'Reprendre la connexion'}
			</button>
		</section>

		<!-- Nothing to change while the headphones are not linked: the controls only show what they said. -->
		{#if !connected}
			<p id="headset-inactive" class="text-sm text-muted">
				Les réglages ci-dessous sont inactifs tant que la connexion au casque n’est pas ouverte.
			</p>
		{/if}
		<fieldset
			disabled={!connected}
			aria-describedby={connected ? undefined : 'headset-inactive'}
			class="grid grid-cols-[repeat(auto-fit,minmax(24rem,1fr))] items-start gap-4 border-0 p-0 disabled:opacity-60"
		>
			<section class="card" aria-labelledby="ambient-title">
				<h3 id="ambient-title" class="mb-2 text-lg font-semibold">Son ambiant</h3>
				{#if ambient}
					<SettingRow label="Mode" description="Ce que le casque fait du bruit extérieur.">
						<SegmentedControl
							label="Mode du son ambiant"
							options={modes}
							value={ambient.mode}
							onchange={setMode}
						/>
					</SettingRow>
					{#if ambient.mode === 'ambient'}
						<SettingRow label="Niveau" description="La part de son extérieur que l’on entend.">
							<RangeControl
								label="Niveau du son ambiant"
								value={ambient.level}
								min={0}
								max={MAX_AMBIENT_LEVEL}
								onchange={(level) => headsets.setAmbient({ ...ambient, level })}
							/>
						</SettingRow>
						<SettingRow label="Voix" description="Laisse passer les voix, atténue le reste.">
							<Switch
								checked={ambient.focusOnVoice}
								label="Laisser passer les voix"
								onchange={(focusOnVoice) => headsets.setAmbient({ ...ambient, focusOnVoice })}
							/>
						</SettingRow>
					{/if}
				{:else}
					<p class="text-sm text-muted">Le casque n’a pas encore dit son réglage.</p>
				{/if}
			</section>

			<section class="card" aria-labelledby="equalizer-title">
				<h3 id="equalizer-title" class="mb-2 text-lg font-semibold">Égaliseur</h3>
				{#if equalizer}
					<SettingRow label="Préréglage" description="Un réglage prêt à l’emploi.">
						<SelectField
							label="Préréglage de l’égaliseur"
							options={presetOptions(equalizer.preset)}
							value={equalizer.preset}
							onchange={setPreset}
						/>
					</SettingRow>
					<SettingRow label="Basses" description="Renfort des basses (Clear Bass).">
						<RangeControl
							label="Égaliseur : basses"
							value={equalizer.clearBass}
							min={EQUALIZER_MIN}
							max={EQUALIZER_MAX}
							onchange={(clearBass) =>
								headsets.setEqualizer({ type: 'manual', clearBass, bands: equalizer.bands })}
						/>
					</SettingRow>
					{#each EQUALIZER_BANDS as band, index (band)}
						<SettingRow label={band} description={`Gain autour de ${band}.`}>
							<RangeControl
								label={`Égaliseur : ${band}`}
								value={equalizer.bands[index]}
								min={EQUALIZER_MIN}
								max={EQUALIZER_MAX}
								onchange={(value) => setBand(index, value)}
							/>
						</SettingRow>
					{/each}
				{:else}
					<p class="text-sm text-muted">Le casque n’a pas encore dit son égaliseur.</p>
				{/if}
			</section>
		</fieldset>
	</div>
{/if}
