<!--
@component
Test harness: provides the manager context the pages read, then shows the page.
-->
<script lang="ts">
	import type { Component } from 'svelte';
	import { ManagerApi } from '#lib/api/manager-api.js';
	import type { ManagerConnection } from '#lib/connection/index.js';
	import type { DeckState } from '#lib/deck/deck-state.svelte.js';
	import { setManager } from '#lib/deck/manager-context.js';

	interface Props {
		deck: DeckState;
		page: Component;
	}

	let { deck, page: Page }: Props = $props();

	setManager({
		get deck() {
			return deck;
		},
		connection: {} as ManagerConnection,
		api: new ManagerApi()
	});
</script>

<Page />
