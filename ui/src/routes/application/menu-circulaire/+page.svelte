<script lang="ts">
	import PageHeader from '#lib/components/PageHeader.svelte';
	import RadialMenuPage from '#lib/components/RadialMenuPage.svelte';
	import { getManager } from '#lib/deck/manager-context.js';

	const { deck } = getManager();
</script>

<PageHeader title="Menu circulaire" />
<RadialMenuPage {deck} mouse={deck.mice.mouse} config={deck.mice.config} />
