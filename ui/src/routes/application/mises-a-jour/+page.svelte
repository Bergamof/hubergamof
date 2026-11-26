<script lang="ts">
	import PageHeader from '#lib/components/PageHeader.svelte';
	import UpdatePanel from '#lib/components/UpdatePanel.svelte';
	import { getManager } from '#lib/deck/manager-context.js';

	const { deck } = getManager();
</script>

<PageHeader title="Mises à jour" />
<UpdatePanel
	version={deck.managerVersion}
	status={deck.updateStatus}
	settings={deck.updateSettings}
	oncheck={() => deck.checkForUpdates()}
	oninstall={() => deck.installUpdate()}
	onautoinstall={(enabled) => deck.setAutoInstall(enabled)}
	oncheckdaily={(enabled) => deck.setUpdateCheck(enabled)}
/>
