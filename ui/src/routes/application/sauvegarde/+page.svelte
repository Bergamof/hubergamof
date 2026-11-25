<script lang="ts">
	import BackupPage from '#lib/components/BackupPage.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { getManager } from '#lib/deck/manager-context.js';

	const { deck, api } = getManager();
</script>

<PageHeader
	title="Sauvegarde"
	description="Exporter la configuration de HuBergamof, la restaurer, ou revenir en arrière."
/>
<BackupPage {api} decks={deck.devices} mice={deck.mice.mice} />
