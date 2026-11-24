<!--
@component
The foot of a form that is sent as a whole: a note while there are changes not sent yet, and the
button that sends them.
-->
<script lang="ts">
	interface Props {
		/** Something was typed that is not saved yet. */
		dirty: boolean;
		submitLabel: string;
	}

	let { dirty, submitLabel }: Props = $props();
</script>

<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
	{#if dirty}
		<p class="text-sm text-muted">Modifications non enregistrées.</p>
	{/if}
	<button type="submit" class="primary-button sm:ml-auto">{submitLabel}</button>
</div>
