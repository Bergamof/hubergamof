<!--
@component
The page of a Stream Deck: the one asked for by the address (`?appareil=`), once its configuration is known. A
device that is no longer connected is said so, instead of silently showing another one.
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { getManager } from '#lib/deck/manager-context.js';
	import { PATHS } from '#lib/deck/nav-model.js';
	import DeckEditor from './DeckEditor.svelte';
	import EmptyState from './EmptyState.svelte';
	import PageHeader from './PageHeader.svelte';

	const { deck, api } = getManager();

	const requested = $derived(page.url.searchParams.get('appareil'));
	const connected = $derived(deck.devices.some((device) => device.serial === requested));
	/** The address names a device that is not (or no longer) connected, while others are. */
	const gone = $derived(requested !== null && deck.devices.length > 0 && !connected);

	// Only a device that is connected is followed; `select` reads what it changes, which must not rerun this.
	$effect(() => {
		if (requested && connected) untrack(() => deck.select(requested));
	});
</script>

{#if gone}
	<PageHeader title="Stream Deck" />
	<EmptyState title="Ce Stream Deck n’est plus connecté" live>
		<p>Sa configuration enregistrée est conservée. Rebranchez-le pour la modifier.</p>
		<a href={resolve(PATHS.devices)} class="link">Voir les périphériques</a>
	</EmptyState>
{:else if deck.device && deck.config}
	<PageHeader title={deck.device.displayName} />
	<DeckEditor {deck} device={deck.device} config={deck.config} {api} />
{:else if deck.devices.length === 0}
	<PageHeader title="Stream Deck" />
	<EmptyState title="Aucun Stream Deck connecté" live>
		<p>Sa configuration enregistrée est conservée. Branchez-le pour la modifier.</p>
		<a href={resolve(PATHS.devices)} class="link">Voir les périphériques</a>
	</EmptyState>
{:else}
	<PageHeader title="Stream Deck" />
	<p role="status" class="m-auto text-muted">Chargement de la configuration…</p>
{/if}
