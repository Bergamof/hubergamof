<!--
@component
Options of the touch strip: what it shows (its colours are the theme of the device). Every change is applied at once.
-->
<script lang="ts">
	import type { StripConfig } from '#lib/generated/index.js';
	import IntegerField from './IntegerField.svelte';

	interface Props {
		strip: StripConfig;
		/** Id of the heading, for the page to bring it into view. */
		headingId: string;
		onchange: (changes: Partial<StripConfig>) => void;
	}

	let { strip, headingId, onchange }: Props = $props();

	const uid = $props.id();

	/** Periods of the system measures, in seconds: `SYSTEM_REFRESH_CHOICES` of `hubergamof_protocol`. */
	const REFRESH_SECONDS = [1, 2, 5, 10] as const;
	/** Range of the alert load, in percent: `LOAD_ALERT_RANGE` of `hubergamof_protocol`. */
	const ALERT_MIN = 50;
	const ALERT_MAX = 100;
</script>

<section aria-labelledby={headingId} class="flex flex-col gap-4">
	<h2 id={headingId} tabindex="-1" class="text-lg font-semibold">Bande tactile</h2>
	<p class="text-sm text-muted">
		La bande affiche l’heure, la charge du système, la page, le volume et le pomodoro. Les valeurs
		de l’aperçu sont des exemples.
	</p>

	<label class="flex items-center gap-2">
		<input
			type="checkbox"
			class="size-5 accent-ring focus-ring"
			checked={strip.showSeconds}
			onchange={(event) => onchange({ showSeconds: event.currentTarget.checked })}
		/>
		Afficher les secondes
	</label>

	<label class="flex items-center gap-2">
		<input
			type="checkbox"
			class="size-5 accent-ring focus-ring"
			checked={strip.showPomodoroProgress}
			onchange={(event) => onchange({ showPomodoroProgress: event.currentTarget.checked })}
		/>
		Afficher la barre de progression du pomodoro
	</label>

	<label class="flex items-center gap-2">
		<input
			type="checkbox"
			class="size-5 accent-ring focus-ring"
			checked={strip.showHistory}
			onchange={(event) => onchange({ showHistory: event.currentTarget.checked })}
		/>
		Afficher l’historique des mesures en courbes
	</label>

	<div class="flex flex-col gap-1">
		<label class="font-medium" for="{uid}-refresh">Actualisation des mesures du système</label>
		<select
			id="{uid}-refresh"
			class="field w-auto"
			value={String(strip.systemRefreshSeconds)}
			onchange={(event) => onchange({ systemRefreshSeconds: Number(event.currentTarget.value) })}
		>
			{#each REFRESH_SECONDS as seconds (seconds)}
				<option value={String(seconds)}>Toutes les {seconds} s</option>
			{/each}
		</select>
	</div>

	<IntegerField
		label="Alerte de charge (en pourcentage)"
		value={strip.loadAlertPercent}
		min={ALERT_MIN}
		max={ALERT_MAX}
		unit="%"
		hint="Une mesure est affichée en rouge à partir de cette charge, entre {ALERT_MIN} et {ALERT_MAX} %."
		error="Saisissez un nombre entier entre {ALERT_MIN} et {ALERT_MAX}."
		onchange={(loadAlertPercent) => onchange({ loadAlertPercent })}
	/>
</section>
