<script lang="ts">
	import HeadsetPage from '#lib/components/HeadsetPage.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { getManager } from '#lib/deck/manager-context.js';

	const { deck } = getManager();
</script>

<PageHeader title={deck.headsets.headset?.name ?? 'Casque'} />
<HeadsetPage {deck} />
