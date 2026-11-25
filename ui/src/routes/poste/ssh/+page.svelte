<script lang="ts">
	import PageHeader from '#lib/components/PageHeader.svelte';
	import SshPanel from '#lib/components/SshPanel.svelte';
	import { getManager } from '#lib/deck/manager-context.js';

	const { deck } = getManager();
</script>

<PageHeader title="SSH" />
<SshPanel {deck} />
