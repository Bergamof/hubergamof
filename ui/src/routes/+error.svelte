<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { PATHS } from '#lib/deck/nav-model.js';
</script>

<svelte:head>
	<title>{page.status === 404 ? 'Page introuvable' : 'Erreur'} · HuBergamof</title>
</svelte:head>

<PageHeader title={page.status === 404 ? 'Page introuvable' : 'Une erreur est survenue'} />
<EmptyState
	title={page.status === 404 ? 'Cette page n’existe pas' : 'La page n’a pas pu être affichée'}
>
	<a href={resolve(PATHS.home)} class="link">Retour à l’accueil</a>
</EmptyState>
