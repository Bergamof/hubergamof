<!--
@component
Test harness: provides the manager context and shows the modules page for a given `?module=`.
-->
<script lang="ts">
	import { ManagerApi } from '#lib/api/manager-api.js';
	import type { ManagerConnection } from '#lib/connection/index.js';
	import type { DeckState } from '#lib/deck/deck-state.svelte.js';
	import { setManager } from '#lib/deck/manager-context.js';
	import ModulesPage from './ModulesPage.svelte';

	interface Props {
		deck: DeckState;
		requested: string | null;
	}

	let { deck, requested }: Props = $props();

	setManager({
		get deck() {
			return deck;
		},
		connection: {} as ManagerConnection,
		api: new ManagerApi()
	});
</script>

<ModulesPage {requested} />
