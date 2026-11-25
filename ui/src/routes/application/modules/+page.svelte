<script lang="ts">
	import ApplicationPanel from '#lib/components/ApplicationPanel.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { getManager } from '#lib/deck/manager-context.js';

	const { deck, api } = getManager();
</script>

<PageHeader title="Modules" />
{#if deck.application.modules.length === 0}
	<EmptyState title="Aucun module dans l’application" />
{:else}
	<ApplicationPanel {deck} {api} />
{/if}
