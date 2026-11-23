<script lang="ts">
	import { tick } from 'svelte';
	import { ManagerApi } from '#lib/api/manager-api.js';
	import { ManagerConnection, managerSocketUrl } from '#lib/connection/index.js';
	import { DeckState } from '#lib/deck/deck-state.svelte.js';
	import DeckEditor from '#lib/components/DeckEditor.svelte';
	import DeviceSelector from '#lib/components/DeviceSelector.svelte';
	import ApplicationPanel from '#lib/components/ApplicationPanel.svelte';
	import MachinePage from '#lib/components/MachinePage.svelte';
	import MouseEditor from '#lib/components/MouseEditor.svelte';
	import RadialMenuPage from '#lib/components/RadialMenuPage.svelte';
	import SegmentedControl from '#lib/components/SegmentedControl.svelte';
	import {
		SECTION_LABELS,
		availableSections,
		sectionShown,
		type Section
	} from '#lib/deck/sections.js';
	import UpdatePanel from '#lib/components/UpdatePanel.svelte';
	import Notices from '#lib/components/Notices.svelte';
	import BrandWordmark from '#lib/components/BrandWordmark.svelte';

	/** Focus target once the last notice is dismissed. */
	const MAIN_ID = 'main-content';

	const api = new ManagerApi();

	let connection = $state<ManagerConnection>();
	let deck = $state<DeckState>();

	/** What the person asked to see; what is shown also depends on what is connected. */
	let wanted = $state<Section>('deck');
	const hasDecks = $derived((deck?.devices.length ?? 0) > 0);
	const hasMice = $derived((deck?.mice.mice.length ?? 0) > 0);
	const hasModules = $derived((deck?.application.modules.length ?? 0) > 0);
	const sections = $derived(availableSections(hasDecks, hasMice, hasModules));
	const section = $derived(sectionShown(wanted, sections));

	const isOnline = $derived(connection?.status === 'open');
	/** The computer is locked: the manager refuses everything, the page is only to be read. */
	const locked = $derived(deck?.application.locked ?? false);
	/** The editor shown: the mouse one, or the deck one, once its configuration is known. */
	const editorShown = $derived(
		section === 'machine'
			? true
			: section === 'modules'
				? hasModules
				: section === 'menu'
					? deck?.mice.radialMenu !== undefined
					: section === 'mouse'
						? deck?.mice.mouse !== undefined && deck.mice.config !== undefined
						: deck?.device !== undefined && deck.config !== undefined
	);
	/** The editor can be used: the connection is up and the computer is not locked. */
	const usable = $derived(isOnline && !locked);
	/** The page then shows its own alert about the lost connection: the header does not repeat it. */
	const offlineAlertShown = $derived(!isOnline && editorShown);

	let editor: HTMLDivElement | undefined = $state();
	/** Focused control of the editor when it became unusable, focused again once usable. */
	let focusBeforeOffline: HTMLElement | undefined;

	// Before the DOM update: the editor turns inert, which drops the focus.
	$effect.pre(() => {
		if (usable || !editor) return;
		const focused = document.activeElement;
		if (focused instanceof HTMLElement && editor.contains(focused)) {
			focusBeforeOffline = focused;
			// The page keeps the focus once the editor is inert, instead of dropping it on the body.
			void tick().then(() => document.getElementById(MAIN_ID)?.focus());
		}
	});

	$effect(() => {
		if (!usable || !focusBeforeOffline) return;
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
		{(section !== 'deck' ? deck?.mice.mouse : deck?.device)
			? `${(section !== 'deck' ? deck?.mice.mouse : deck?.device)?.displayName} · HuBergamof`
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
			{#if locked}
				<p
					role="alert"
					class="rounded-xl border border-warning bg-warning-soft p-3 text-sm text-warning"
				>
					L’ordinateur est verrouillé : l’interface est en lecture seule jusqu’au déverrouillage, le
					gestionnaire refuse toute action.
				</p>
			{/if}
			<div bind:this={editor} class="flex flex-col gap-6" inert={!usable}>
				{#if sections.length > 1}
					<div role="group" aria-label="Rubriques de la configuration" class="flex flex-wrap gap-2">
						{#each sections as choice (choice)}
							<button
								type="button"
								class={[
									'rounded-full border px-4 py-1.5 font-medium focus-ring',
									section === choice
										? 'border-accent bg-accent text-on-accent'
										: 'border-line bg-surface text-muted'
								]}
								aria-pressed={section === choice}
								onclick={() => (wanted = choice)}
							>
								{SECTION_LABELS[choice]}
							</button>
						{/each}
					</div>
				{/if}
				{#if section === 'modules' || section === 'machine'}
					<!-- The modules of the application and the machine belong to no device. -->
				{:else if section !== 'deck' && deck.mice.mouse}
					{#if deck.mice.mice.length > 1}
						<SegmentedControl
							label="Souris"
							options={deck.mice.mice.map((mouse) => ({
								value: mouse.serial,
								label: mouse.displayName
							}))}
							value={deck.mice.selectedSerial ?? ''}
							onchange={(serial) => deck?.mice.select(serial)}
						/>
					{/if}
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
				{#if section === 'machine'}
					<MachinePage {deck} />
				{:else if section === 'modules'}
					<ApplicationPanel {deck} {api} />
				{:else if section === 'menu'}
					<RadialMenuPage {deck} mouse={deck.mice.mouse} config={deck.mice.config} />
				{:else if section === 'mouse' && deck.mice.mouse && deck.mice.config}
					<MouseEditor {deck} mouse={deck.mice.mouse} config={deck.mice.config} />
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
		<footer inert={locked}>
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
