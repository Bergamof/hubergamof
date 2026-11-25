<script lang="ts">
	import './layout.css';
	import favicon from '#lib/assets/favicon.svg';
	import { ManagerApi } from '#lib/api/manager-api.js';
	import AppShell from '#lib/components/AppShell.svelte';
	import { ManagerConnection, managerSocketUrl } from '#lib/connection/index.js';
	import { DeckState } from '#lib/deck/deck-state.svelte.js';
	import { setManager } from '#lib/deck/manager-context.js';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	// The page is rendered in the browser only (`ssr = false`), where the address of the manager is known.
	const connection = new ManagerConnection({ url: managerSocketUrl(location) });
	const deck = new DeckState(connection);
	setManager({ deck, connection, api: new ManagerApi() });

	$effect(() => {
		const stopListening = deck.listen();
		connection.connect();
		return () => {
			stopListening();
			connection.disconnect();
		};
	});
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
</svelte:head>

<AppShell {children} />
