<!--
@component
The page of a mouse: the one asked for by the address (`?appareil=`), once its configuration is known. A mouse
that is no longer connected is said so, instead of silently showing another one.
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { getManager } from '#lib/deck/manager-context.js';
	import { PATHS } from '#lib/deck/nav-model.js';
	import EmptyState from './EmptyState.svelte';
	import MouseEditor from './MouseEditor.svelte';
	import PageHeader from './PageHeader.svelte';

	const { deck } = getManager();

	const requested = $derived(page.url.searchParams.get('appareil'));
	const connected = $derived(deck.mice.mice.some((mouse) => mouse.serial === requested));
	/** The address names a mouse that is not (or no longer) connected, while others are. */
	const gone = $derived(requested !== null && deck.mice.mice.length > 0 && !connected);

	// Only a mouse that is connected is followed; `select` reads what it changes, which must not rerun this.
	$effect(() => {
		if (requested && connected) untrack(() => deck.mice.select(requested));
	});
</script>

{#if gone}
	<PageHeader title="Souris" />
	<EmptyState title="Cette souris n’est plus connectée" live>
		<p>Sa configuration enregistrée est conservée. Rallumez-la pour la modifier.</p>
		<a href={resolve(PATHS.devices)} class="link">Voir les périphériques</a>
	</EmptyState>
{:else if deck.mice.mouse && deck.mice.config}
	<h1 class="sr-only">{deck.mice.mouse.displayName}</h1>
	<MouseEditor {deck} mouse={deck.mice.mouse} config={deck.mice.config} />
{:else if deck.mice.mice.length === 0}
	<PageHeader title="Souris" />
	<EmptyState title="Aucune souris connectée" live>
		<p>Allumez-la et branchez le récepteur Logi Bolt.</p>
		<a href={resolve(PATHS.devices)} class="link">Voir les périphériques</a>
	</EmptyState>
{:else}
	<PageHeader title="Souris" />
	<p role="status" class="m-auto text-muted">Chargement de la configuration…</p>
{/if}
