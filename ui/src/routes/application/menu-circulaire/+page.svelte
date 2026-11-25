<script lang="ts">
	import PageHeader from '#lib/components/PageHeader.svelte';
	import RadialMenuPage from '#lib/components/RadialMenuPage.svelte';
	import { getManager } from '#lib/deck/manager-context.js';

	const { deck } = getManager();
</script>

<PageHeader title="Menu circulaire" />
{#if deck.mice.radialMenu}
	<RadialMenuPage {deck} mouse={deck.mice.mouse} config={deck.mice.config} />
{:else}
	<p class="m-auto text-muted">Chargement de la configuration…</p>
{/if}
