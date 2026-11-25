<script lang="ts">
	import { page } from '$app/state';
	import ModulesPage from '#lib/components/ModulesPage.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
</script>

<PageHeader title="Modules" description="Les réglages des modules de l’application." />
<ModulesPage requested={page.url.searchParams.get('module')} />
