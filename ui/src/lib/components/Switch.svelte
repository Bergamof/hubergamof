<!--
@component
An on/off switch with its label: a checkbox announced as a switch. `onchange` tells what was chosen: the owner decides when it is sent (at once on the mouse pages, with the
form on the Git ones). `indeterminate` shows some of the things it stands for on and some off
(a switch for a whole module); pressing it then sets all of them.
-->
<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		checked: boolean;
		/** Some on, some off: the thumb stops half way. */
		indeterminate?: boolean;
		disabled?: boolean;
		/** Name read by assistive technologies when the visible label is not enough (or there is none). */
		label?: string;
		/** Id of what explains the switch (why it cannot change, for instance). */
		describedby?: string;
		onchange: (checked: boolean) => void;
		/** The visible label, after the switch. */
		children?: Snippet;
	}

	let {
		checked,
		indeterminate = false,
		disabled = false,
		label,
		describedby,
		onchange,
		children
	}: Props = $props();
</script>

<label class="flex items-center gap-2.5">
	<input
		type="checkbox"
		role="switch"
		class="switch"
		{checked}
		{indeterminate}
		{disabled}
		aria-label={label}
		aria-describedby={describedby}
		onchange={(event) => onchange(event.currentTarget.checked)}
	/>
	{#if children}
		<span>{@render children()}</span>
	{/if}
</label>
