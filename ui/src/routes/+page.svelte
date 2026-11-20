<script lang="ts">
	import { tick } from 'svelte';
	import { ManagerApi } from '#lib/api/manager-api.js';
	import { ManagerConnection, managerSocketUrl } from '#lib/connection/index.js';
	import { DeckState } from '#lib/deck/deck-state.svelte.js';
	import DeckEditor from '#lib/components/DeckEditor.svelte';
	import DeviceSelector from '#lib/components/DeviceSelector.svelte';
	import MouseEditor from '#lib/components/MouseEditor.svelte';
	import UpdatePanel from '#lib/components/UpdatePanel.svelte';
	import Notices from '#lib/components/Notices.svelte';
	import BrandWordmark from '#lib/components/BrandWordmark.svelte';

	/** Focus target once the last notice is dismissed. */
	const MAIN_ID = 'main-content';

	const api = new ManagerApi();

	let connection = $state<ManagerConnection>();
	let deck = $state<DeckState>();

	type Section = 'deck' | 'mouse';
	/** What the person asked to see; what is shown also depends on what is connected. */
	let wanted = $state<Section>('deck');
	const hasDecks = $derived((deck?.devices.length ?? 0) > 0);
	const hasMice = $derived((deck?.mice.mice.length ?? 0) > 0);
	const section: Section = $derived(
		hasMice && (wanted === 'mouse' || !hasDecks) ? 'mouse' : 'deck'
	);
	const SECTIONS: readonly { value: Section; label: string }[] = [
		{ value: 'deck', label: 'Stream Deck' },
		{ value: 'mouse', label: 'Souris' }
	];

	const isOnline = $derived(connection?.status === 'open');
	/** The editor shown: the mouse one, or the deck one, once its configuration is known. */
	const editorShown = $derived(
		section === 'mouse'
			? deck?.mice.mouse !== undefined && deck.mice.config !== undefined
			: deck?.device !== undefined && deck.config !== undefined
	);
	/** The page then shows its own alert about the lost connection: the header does not repeat it. */
	const offlineAlertShown = $derived(!isOnline && editorShown);

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
	<title>
		{(section === 'mouse' ? deck?.mice.mouse : deck?.device)
			? `${(section === 'mouse' ? deck?.mice.mouse : deck?.device)?.displayName} · HuBergamof`
			: 'HuBergamof'}
	</title>
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
			<BrandWordmark class="h-10 w-auto text-fg" />
			<span class="sr-only">HuBergamof</span>
		</h1>
		<p
			role="status"
			aria-live={offlineAlertShown ? 'off' : 'polite'}
			class="flex min-h-9 items-center gap-2 rounded-full border border-line bg-surface px-3.5 text-sm"
		>
			<span aria-hidden="true" class={['size-2.5 rounded-full', isOnline ? 'bg-ok' : 'bg-warning']}
			></span>
			{isOnline ? 'Connecté au gestionnaire' : 'Non connecté au gestionnaire'}
		</p>
	</header>

	<main id={MAIN_ID} tabindex="-1" class="flex flex-1 flex-col gap-6 outline-none">
		{#if deck && editorShown}
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
				{#if hasDecks && hasMice}
					<nav aria-label="Appareils à configurer" class="flex gap-2">
						{#each SECTIONS as choice (choice.value)}
							<button
								type="button"
								class={[
									'rounded-full border px-4 py-1.5 font-medium focus-ring',
									section === choice.value
										? 'border-accent bg-accent text-on-accent'
										: 'border-line bg-surface text-muted'
								]}
								aria-current={section === choice.value ? 'page' : undefined}
								onclick={() => (wanted = choice.value)}
							>
								{choice.label}
							</button>
						{/each}
					</nav>
				{/if}
				{#if section === 'mouse' && deck.mice.mouse && deck.mice.config}
					<DeviceSelector
						devices={deck.mice.mice}
						selected={deck.mice.selectedSerial}
						label="Souris"
						onselect={(serial) => deck?.mice.select(serial)}
					/>
				{:else if deck.device}
					<DeviceSelector
						devices={deck.devices}
						selected={deck.selectedSerial}
						onselect={(serial) => deck?.select(serial)}
					/>
				{/if}
				<Notices
					notices={deck.notices}
					fallbackFocusId={MAIN_ID}
					ondismiss={(id) => deck?.dismiss(id)}
				/>
				{#if section === 'mouse' && deck.mice.mouse && deck.mice.config}
					<MouseEditor
						{deck}
						mouse={deck.mice.mouse}
						config={deck.mice.config}
						radialMenu={deck.mice.radialMenu}
					/>
				{:else if deck.device && deck.config}
					<DeckEditor {deck} device={deck.device} config={deck.config} {api} />
				{/if}
			</div>
		{:else if !isOnline || !deck}
			{@render hero(
				'Connexion au gestionnaire…',
				'Si rien ne se passe, vérifiez que le service hubergamof est lancé.'
			)}
		{:else if !hasDecks && !hasMice}
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

	<!-- Only while connected: the version and the check belong to the manager answering. -->
	{#if deck && isOnline}
		<footer>
			<UpdatePanel
				version={deck.managerVersion}
				status={deck.updateStatus}
				settings={deck.updateSettings}
				oncheck={() => deck?.checkForUpdates()}
				oninstall={() => deck?.installUpdate()}
				onautoinstall={(enabled) => deck?.setAutoInstall(enabled)}
			/>
		</footer>
	{/if}
</div>
