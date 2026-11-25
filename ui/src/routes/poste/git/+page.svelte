<script lang="ts">
	import GitPanel from '#lib/components/GitPanel.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { getManager } from '#lib/deck/manager-context.js';

	const { deck } = getManager();
</script>

<PageHeader title="Git" />
<GitPanel {deck} />
