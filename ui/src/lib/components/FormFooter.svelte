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
	<!-- The region stays in the page, empty: a note inserted with its region is read unreliably. -->
	<p role="status" class="text-sm text-muted empty:hidden">
		{#if dirty}Modifications non enregistrées.{/if}
	</p>
	<button type="submit" class="primary-button sm:ml-auto">{submitLabel}</button>
</div>
