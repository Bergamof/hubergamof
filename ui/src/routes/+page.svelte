<script lang="ts">
	import { ManagerApi } from '#lib/api/manager-api.js';
	import { ManagerConnection, managerSocketUrl } from '#lib/connection/index.js';
	import { DeckState } from '#lib/deck/deck-state.svelte.js';
	import DeckEditor from '#lib/components/DeckEditor.svelte';
	import DeviceSelector from '#lib/components/DeviceSelector.svelte';
	import Notices from '#lib/components/Notices.svelte';

	/** Focus target once the last notice is dismissed. */
	const MAIN_ID = 'main-content';

	const api = new ManagerApi();

	let connection = $state<ManagerConnection>();
	let deck = $state<DeckState>();

	const isOnline = $derived(connection?.status === 'open');

	$effect(() => {
		const managerConnection = new ManagerConnection({ url: managerSocketUrl(location) });
		const deckState = new DeckState(managerConnection);
		const stopListening = deckState.listen();
		managerConnection.connect();
		connection = managerConnection;
		deck = deckState;

		return () => {
			stopListening();
			managerConnection.disconnect();
		};
	});
</script>

<svelte:head>
	<title>Bergadeck</title>
</svelte:head>

<div class="mx-auto flex min-h-screen max-w-6xl flex-col gap-6 p-6">
	<header class="flex flex-wrap items-center justify-between gap-4">
		<h1 class="text-2xl font-bold tracking-tight">Bergadeck</h1>
		<p role="status" aria-live="polite" class="flex items-center gap-2 text-sm">
			<span
				aria-hidden="true"
				class={[
					'size-2.5 rounded-full',
					isOnline ? 'bg-emerald-600 dark:bg-emerald-400' : 'bg-amber-600 dark:bg-amber-400'
				]}
			></span>
			{isOnline ? 'Connecté au gestionnaire' : 'Non connecté au gestionnaire'}
		</p>
	</header>

	<main id={MAIN_ID} tabindex="-1" class="flex flex-1 flex-col gap-6 outline-none">
		{#if deck?.device && deck.config}
			{#if !isOnline}
				<p
					role="alert"
					class="rounded-lg border border-amber-600 bg-amber-50 p-3 text-sm text-amber-950 dark:border-amber-400 dark:bg-amber-950 dark:text-amber-100"
				>
					Connexion au gestionnaire perdue, reconnexion en cours… La configuration reste affichée
					mais ne peut pas être modifiée.
				</p>
			{/if}
			<div class="flex flex-col gap-6" inert={!isOnline}>
				<DeviceSelector
					devices={deck.devices}
					selected={deck.selectedSerial}
					onselect={(serial) => deck?.select(serial)}
				/>
				<Notices
					notices={deck.notices}
					fallbackFocusId={MAIN_ID}
					ondismiss={(id) => deck?.dismiss(id)}
				/>
				<DeckEditor {deck} device={deck.device} config={deck.config} {api} />
			</div>
		{:else if !isOnline || !deck}
			<div class="m-auto flex flex-col items-center gap-2 text-center">
				<p class="text-zinc-600 dark:text-zinc-400">Connexion au gestionnaire…</p>
				<p class="text-sm text-zinc-600 dark:text-zinc-400">
					Si rien ne se passe, vérifiez que le service bergadeck est lancé.
				</p>
			</div>
		{:else if deck.devices.length === 0}
			<p class="m-auto text-zinc-600 dark:text-zinc-400">Aucun appareil connecté</p>
		{:else}
			<Notices
				notices={deck.notices}
				fallbackFocusId={MAIN_ID}
				ondismiss={(id) => deck?.dismiss(id)}
			/>
			<p class="m-auto text-zinc-600 dark:text-zinc-400">Chargement de la configuration…</p>
		{/if}
	</main>
</div>
