<script lang="ts">
	import PageHeader from '#lib/components/PageHeader.svelte';
	import ShellAliasesPanel from '#lib/components/ShellAliasesPanel.svelte';
	import { getManager } from '#lib/deck/manager-context.js';

	const { deck } = getManager();
</script>

<PageHeader title="Alias du shell" />
<ShellAliasesPanel {deck} />
