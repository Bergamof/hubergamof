<script lang="ts">
	import { tick } from 'svelte';
	import { ManagerApi } from '#lib/api/manager-api.js';
	import { ManagerConnection, managerSocketUrl } from '#lib/connection/index.js';
	import { DeckState } from '#lib/deck/deck-state.svelte.js';
	import DeckEditor from '#lib/components/DeckEditor.svelte';
	import DeviceSelector from '#lib/components/DeviceSelector.svelte';
	import Notices from '#lib/components/Notices.svelte';
	import banner from '#lib/assets/banner.svg';

	/** Focus target once the last notice is dismissed. */
	const MAIN_ID = 'main-content';

	const api = new ManagerApi();

	let connection = $state<ManagerConnection>();
	let deck = $state<DeckState>();

	const isOnline = $derived(connection?.status === 'open');

	let editor: HTMLDivElement | undefined = $state();
	/** Focused control of the editor when the connection was lost, focused again once back. */
	let focusBeforeOffline: HTMLElement | undefined;

	// Before the DOM update: the editor turns inert, which drops the focus.
	$effect.pre(() => {
		if (isOnline || !editor) return;
		const focused = document.activeElement;
		if (focused instanceof HTMLElement && editor.contains(focused)) focusBeforeOffline = focused;
	});

	$effect(() => {
		if (!isOnline || !focusBeforeOffline) return;
		const target = focusBeforeOffline;
		focusBeforeOffline = undefined;
		void tick().then(() => {
			const fallback = document.getElementById(MAIN_ID);
			(target.isConnected ? target : fallback)?.focus();
		});
	});

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
	<title>HuBergamof</title>
</svelte:head>

{#snippet hero(message: string, detail?: string)}
	<div class="m-auto flex flex-col items-center gap-2 text-center">
		<p class="font-display text-xl font-semibold">{message}</p>
		{#if detail}
			<p class="text-sm text-muted">{detail}</p>
		{/if}
	</div>
{/snippet}

<div class="mx-auto flex min-h-screen max-w-6xl flex-col gap-6 px-4 py-5 sm:px-6">
	<header class="flex flex-wrap items-center justify-between gap-x-4 gap-y-3">
		<h1>
			<img
				src={banner}
				alt="HuBergamof"
				width="1280"
				height="320"
				class="h-20 w-auto rounded-xl border border-line sm:h-24"
			/>
		</h1>
		<p
			role="status"
			aria-live="polite"
			class="flex min-h-9 items-center gap-2 rounded-full border border-line bg-surface px-3.5 text-sm"
		>
			<span aria-hidden="true" class={['size-2.5 rounded-full', isOnline ? 'bg-ok' : 'bg-warning']}
			></span>
			{isOnline ? 'Connecté au gestionnaire' : 'Non connecté au gestionnaire'}
		</p>
	</header>

	<main id={MAIN_ID} tabindex="-1" class="flex flex-1 flex-col gap-6 outline-none">
		{#if deck?.device && deck.config}
			{#if !isOnline}
				<p
					role="alert"
					class="rounded-xl border border-warning bg-warning-soft p-3 text-sm text-warning"
				>
					Connexion au gestionnaire perdue, reconnexion en cours… La configuration reste affichée
					mais ne peut pas être modifiée.
				</p>
			{/if}
			<div bind:this={editor} class="flex flex-col gap-6" inert={!isOnline}>
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
			{@render hero(
				'Connexion au gestionnaire…',
				'Si rien ne se passe, vérifiez que le service hubergamof est lancé.'
			)}
		{:else if deck.devices.length === 0}
			{@render hero('Aucun appareil connecté')}
		{:else}
			<Notices
				notices={deck.notices}
				fallbackFocusId={MAIN_ID}
				ondismiss={(id) => deck?.dismiss(id)}
			/>
			<p class="m-auto text-muted">Chargement de la configuration…</p>
		{/if}
	</main>
</div>
