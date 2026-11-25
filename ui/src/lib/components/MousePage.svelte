<!--
@component
The page of a mouse: the one asked for by the address (`?appareil=`), once its configuration is known.
-->
<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { getManager } from '#lib/deck/manager-context.js';
	import { PATHS } from '#lib/deck/nav-model.js';
	import EmptyState from './EmptyState.svelte';
	import MouseEditor from './MouseEditor.svelte';

	const { deck } = getManager();

	$effect(() => {
		const serial = page.url.searchParams.get('appareil');
		if (serial) deck.mice.select(serial);
	});
</script>

{#if deck.mice.mouse && deck.mice.config}
	<h1 class="sr-only">{deck.mice.mouse.displayName}</h1>
	<MouseEditor {deck} mouse={deck.mice.mouse} config={deck.mice.config} />
{:else if deck.mice.mice.length === 0}
	<h1 class="text-2xl font-semibold">Souris</h1>
	<EmptyState title="Aucune souris connectée">
		<p>Allumez-la et branchez le récepteur Logi Bolt.</p>
		<a href={resolve(PATHS.devices)}>Voir les périphériques</a>
	</EmptyState>
{:else}
	<h1 class="text-2xl font-semibold">Souris</h1>
	<p class="m-auto text-muted">Chargement de la configuration…</p>
{/if}
