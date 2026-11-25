<!--
@component
The page of a Stream Deck: the one asked for by the address (`?appareil=`), once its configuration is known.
-->
<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { getManager } from '#lib/deck/manager-context.js';
	import { PATHS } from '#lib/deck/nav-model.js';
	import DeckEditor from './DeckEditor.svelte';
	import EmptyState from './EmptyState.svelte';
	import PageHeader from './PageHeader.svelte';

	const { deck, api } = getManager();

	// The address names the device; without it, the one that was followed.
	$effect(() => {
		const serial = page.url.searchParams.get('appareil');
		if (serial) deck.select(serial);
	});
</script>

{#if deck.device && deck.config}
	<PageHeader title={deck.device.displayName} />
	<DeckEditor {deck} device={deck.device} config={deck.config} {api} />
{:else if deck.devices.length === 0}
	<PageHeader title="Stream Deck" />
	<EmptyState title="Aucun Stream Deck connecté">
		<p>Sa configuration enregistrée est conservée. Branchez-le pour la modifier.</p>
		<a href={resolve(PATHS.devices)}>Voir les périphériques</a>
	</EmptyState>
{:else}
	<PageHeader title="Stream Deck" />
	<p class="m-auto text-muted">Chargement de la configuration…</p>
{/if}
