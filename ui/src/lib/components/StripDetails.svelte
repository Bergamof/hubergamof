<!--
@component
Options of the touch strip: what it shows (its colours are the theme of the device). Every change is applied at once.
-->
<script lang="ts">
	import type { StripConfig } from '#lib/generated/index.js';

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

	let alertDraft: string | undefined = $state();
	const alertText = $derived(alertDraft ?? String(strip.loadAlertPercent));
	const alertValue = $derived(Number(alertText));
	const alertValid = $derived(
		alertText.trim() !== '' &&
			Number.isInteger(alertValue) &&
			alertValue >= ALERT_MIN &&
			alertValue <= ALERT_MAX
	);

	function setAlert(text: string) {
		alertDraft = text;
		const value = Number(text);
		if (text.trim() !== '' && Number.isInteger(value) && value >= ALERT_MIN && value <= ALERT_MAX) {
			onchange({ loadAlertPercent: value });
		}
	}
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

	<div class="flex flex-col gap-1">
		<label class="font-medium" for="{uid}-alert">Alerte de charge (en pourcentage)</label>
		<span class="flex items-center gap-2">
			<input
				id="{uid}-alert"
				type="text"
				inputmode="numeric"
				class="field w-24"
				value={alertText}
				oninput={(event) => setAlert(event.currentTarget.value)}
				onblur={() => (alertDraft = undefined)}
				aria-describedby="{uid}-alert-hint{alertValid ? '' : ` ${uid}-alert-error`}"
				aria-invalid={alertValid ? undefined : true}
			/>
			<span class="text-sm text-muted">%</span>
		</span>
		<p id="{uid}-alert-hint" class="text-sm text-muted">
			Une mesure est affichée en rouge à partir de cette charge, entre {ALERT_MIN} et {ALERT_MAX} %.
		</p>
		{#if !alertValid}
			<p id="{uid}-alert-error" role="alert" class="text-sm font-medium text-danger">
				Saisissez un nombre entier entre {ALERT_MIN} et {ALERT_MAX}.
			</p>
		{/if}
	</div>
</section>
