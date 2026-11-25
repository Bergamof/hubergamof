<script lang="ts">
	import EmptyState from '#lib/components/EmptyState.svelte';
	import ExportImport from '#lib/components/ExportImport.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { getManager } from '#lib/deck/manager-context.js';

	const { deck, api } = getManager();
</script>

<PageHeader title="Sauvegarde" />
{#if deck.device}
	<ExportImport device={deck.device} {api} />
{:else}
	<EmptyState title="Aucun Stream Deck connecté">
		<p>La sauvegarde porte pour l’instant sur la configuration d’un Stream Deck connecté.</p>
	</EmptyState>
{/if}
