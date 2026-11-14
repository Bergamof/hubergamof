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

	/** Delay before the locked device goes dark, in seconds: `LOCK_SLEEP_RANGE` of `hubergamof_protocol`. */
	const LOCK_MIN = 0;
	const LOCK_MAX = 300;

	/** The integer written in `text` if it is within `min` and `max`. */
	function integerWithin(text: string, min: number, max: number): number | undefined {
		const value = Number(text);
		return text.trim() !== '' && Number.isInteger(value) && value >= min && value <= max
			? value
			: undefined;
	}

	let alertDraft: string | undefined = $state();
	const alertText = $derived(alertDraft ?? String(strip.loadAlertPercent));
	const alertValid = $derived(integerWithin(alertText, ALERT_MIN, ALERT_MAX) !== undefined);

	function setAlert(text: string) {
		alertDraft = text;
		const value = integerWithin(text, ALERT_MIN, ALERT_MAX);
		if (value !== undefined) {
			onchange({ loadAlertPercent: value });
		}
	}

	let lockDraft: string | undefined = $state();
	const lockText = $derived(lockDraft ?? String(strip.lockSleepSeconds));
	const lockValid = $derived(integerWithin(lockText, LOCK_MIN, LOCK_MAX) !== undefined);

	function setLock(text: string) {
		lockDraft = text;
		const value = integerWithin(text, LOCK_MIN, LOCK_MAX);
		if (value !== undefined) {
			onchange({ lockSleepSeconds: value });
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

	<div class="flex flex-col gap-1">
		<label class="font-medium" for="{uid}-lock">Extinction au verrouillage de l’ordinateur</label>
		<span class="flex items-center gap-2">
			<input
				id="{uid}-lock"
				type="text"
				inputmode="numeric"
				class="field w-24"
				value={lockText}
				oninput={(event) => setLock(event.currentTarget.value)}
				onblur={() => (lockDraft = undefined)}
				aria-describedby="{uid}-lock-hint{lockValid ? '' : ` ${uid}-lock-error`}"
				aria-invalid={lockValid ? undefined : true}
			/>
			<span class="text-sm text-muted">s</span>
		</span>
		<p id="{uid}-lock-hint" class="text-sm text-muted">
			Ordinateur verrouillé, le Stream Deck s’éteint après ce délai ; 0 le laisse allumé. Une touche
			ou une molette le rallume sans rien déclencher, et aucune action n’est possible tant que
			l’ordinateur est verrouillé.
		</p>
		{#if !lockValid}
			<p id="{uid}-lock-error" role="alert" class="text-sm font-medium text-danger">
				Saisissez un nombre entier de secondes entre {LOCK_MIN} et {LOCK_MAX}.
			</p>
		{/if}
	</div>
</section>
