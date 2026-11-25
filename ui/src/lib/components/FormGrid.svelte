<!--
@component
The grid of a form: twelve columns once the form is wide enough (a container query, so the width of the form counts,
not that of the window), one column before. Its cells say how many columns they take with `FormCell`.
-->
<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		children: Snippet;
		/** The form is a `<form>` named by `label` or `labelledby`, and it submits through `onsubmit`. */
		onsubmit?: (event: SubmitEvent) => void;
		label?: string;
		labelledby?: string;
		describedby?: string;
	}

	let { children, onsubmit, label, labelledby, describedby }: Props = $props();
</script>

<div class="@container">
	{#if onsubmit}
		<form
			novalidate
			aria-label={label}
			aria-labelledby={labelledby}
			aria-describedby={describedby}
			class="grid grid-cols-12 gap-x-3 gap-y-3"
			{onsubmit}
		>
			{@render children()}
		</form>
	{:else}
		<div class="grid grid-cols-12 gap-x-3 gap-y-3">
			{@render children()}
		</div>
	{/if}
</div>
