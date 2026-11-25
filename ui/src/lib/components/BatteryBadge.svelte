<!--
@component
The battery of a device: the icon and the charge, warm and bold when low, with a bolt when charging. The
state is also said in words (for the screen reader always, on the screen with `labelled`): colour alone
never tells it. A battery not read yet shows `unknown` when it is given, else nothing.
-->
<script lang="ts">
	import { isLow, type BatteryReading } from '#lib/deck/battery.js';
	import Icon from './Icon.svelte';

	interface Props {
		battery: BatteryReading | undefined;
		/** Said when the battery is not known (« Non lu »); nothing is shown when left out. */
		unknown?: string;
		/** The state (charging, low) is written on the screen as well. */
		labelled?: boolean;
		class?: string;
	}

	let { battery, unknown, labelled = false, class: className }: Props = $props();

	const low = $derived(battery !== undefined && isLow(battery));
	const state = $derived(battery?.charging ? 'en charge' : low ? 'faible' : '');
</script>

{#if battery}
	<span
		class={[
			'inline-flex items-center gap-1 tabular-nums',
			low ? 'font-semibold text-warning' : 'text-muted',
			className
		]}
	>
		<Icon name="battery" class="size-3.5" />
		{#if battery.charging}<Icon name="bolt" class="-ml-0.5 size-3" />{/if}
		<span>{battery.percent}&nbsp;%</span>
		{#if state}
			<span class={labelled ? '' : 'sr-only'}>{labelled ? `· ${state}` : `, ${state}`}</span>
		{/if}
	</span>
{:else if unknown}
	<span class={['text-muted', className]}>{unknown}</span>
{/if}
