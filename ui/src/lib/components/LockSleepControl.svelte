<!--
@component
Delay before the device goes dark while the computer is locked.
-->
<script lang="ts">
	import { MAX_LOCK_SLEEP_SECONDS } from '#lib/deck/config-edits.js';
	import IntegerField from './IntegerField.svelte';

	interface Props {
		/** Seconds; 0 keeps the device lit. */
		seconds: number;
		onchange: (seconds: number) => void;
	}

	let { seconds, onchange }: Props = $props();

	/** Range of the delay: `LOCK_SLEEP_RANGE` of `hubergamof_protocol`. */
	const MIN_SECONDS = 0;
	const MAX_SECONDS = MAX_LOCK_SLEEP_SECONDS;
</script>

<!-- Capped: the column of the device is as wide as its widest content, and the hint is long. -->
<div class="max-w-sm">
	<IntegerField
		label="Extinction quand l’ordinateur est verrouillé"
		value={seconds}
		min={MIN_SECONDS}
		max={MAX_SECONDS}
		unit="s"
		hint="Le Stream Deck s’éteint après ce délai, entre {MIN_SECONDS} et {MAX_SECONDS} s ; 0 le laisse allumé. Une touche ou une molette le rallume sans rien déclencher. Verrouillé, seuls les modules prévus pour (notifications, pomodoro) restent utilisables ; les autres touches et molettes portent un cadenas."
		error="Saisissez un nombre entier de secondes entre {MIN_SECONDS} et {MAX_SECONDS}."
		{onchange}
	/>
</div>
