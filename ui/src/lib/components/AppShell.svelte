<!--
@component
The frame of every page: the navigation, the bar with the breadcrumb and the connection, the banner when
the page cannot be used, then the page. The navigation stays usable when the page cannot (connection lost,
computer locked): the page is only disabled, not removed, so it stays readable.
-->
<script lang="ts">
	import { tick, type Snippet } from 'svelte';
	import { afterNavigate } from '$app/navigation';
	import { page } from '$app/state';
	import { getManager } from '#lib/deck/manager-context.js';
	import { navDevices, pageOf } from '#lib/deck/nav-model.js';
	import Icon from './Icon.svelte';
	import Notices from './Notices.svelte';
	import SideNav from './SideNav.svelte';
	import StatusBanner from './StatusBanner.svelte';

	/** Focus target after a navigation, and once the last notice is dismissed. */
	const MAIN_ID = 'main-content';

	let { children }: { children: Snippet } = $props();

	const { deck, connection } = getManager();

	const online = $derived(connection.status === 'open');
	const locked = $derived(deck.application.locked);
	/** The page can be used: the connection is up and the computer is not locked. */
	const usable = $derived(online && !locked);
	/** The manager has greeted us: until then there is nothing to show. */
	const greeted = $derived(deck.managerVersion !== undefined);

	const current = $derived(pageOf(page.url.pathname));
	const devices = $derived(navDevices(deck.devices, deck.mice.mice, deck.headsets.headsets));
	const updateAvailable = $derived(deck.updateStatus.type === 'available');

	let content: HTMLFieldSetElement | undefined = $state();
	/** Focused control of the page when it became unusable, focused again once usable. */
	let focusBeforeOffline: HTMLElement | undefined;

	// Before the DOM update: disabling the controls drops the focus.
	$effect.pre(() => {
		if (usable || !content) return;
		const focused = document.activeElement;
		if (focused instanceof HTMLElement && content.contains(focused)) {
			focusBeforeOffline = focused;
			// The page keeps the focus instead of dropping it on the body.
			void tick().then(() => document.getElementById(MAIN_ID)?.focus());
		}
	});

	$effect(() => {
		if (!usable || !focusBeforeOffline) return;
		const target = focusBeforeOffline;
		focusBeforeOffline = undefined;
		void tick().then(() => {
			(target.isConnected ? target : document.getElementById(MAIN_ID))?.focus();
		});
	});

	// A new page starts at its top, with the focus on it: its title is read, not the end of the previous page.
	afterNavigate(({ type }) => {
		if (type === 'enter') return;
		window.scrollTo(0, 0);
		document.getElementById(MAIN_ID)?.focus({ preventScroll: true });
	});
</script>

<svelte:head>
	<title>{current.title} · HuBergamof</title>
</svelte:head>

<a
	href="#{MAIN_ID}"
	class="sr-only z-50 rounded-lg bg-accent px-3 py-2 text-on-accent focus:not-sr-only focus:fixed focus:top-2 focus:left-2"
>
	Aller au contenu
</a>

<div class="grid min-h-screen min-w-[64rem] grid-cols-[15.5rem_minmax(0,1fr)]">
	<aside
		class="sticky top-0 flex h-screen flex-col gap-4 overflow-y-auto border-r border-line bg-surface p-2"
	>
		<SideNav
			pathname={page.url.pathname}
			{devices}
			selectedDeck={deck.selectedSerial}
			selectedMouse={deck.mice.selectedSerial}
			mouseBattery={deck.mice.battery?.percent}
			{updateAvailable}
		/>
		{#if deck.managerVersion}
			<p class="mt-auto border-t border-line px-2 pt-3 text-xs text-muted">
				Version {deck.managerVersion}
			</p>
		{/if}
	</aside>

	<div class="flex min-w-0 flex-col">
		<header
			class="sticky top-0 z-30 flex h-12 items-center gap-3 border-b border-line bg-page/90 px-6 backdrop-blur"
		>
			<nav aria-label="Fil d’Ariane" class="flex min-w-0 flex-1 items-center gap-1.5 text-muted">
				{#if current.group}
					<span>{current.group}</span>
					<Icon name="chevronRight" class="size-3.5" />
				{/if}
				<span class="truncate font-semibold text-fg" aria-current="page">{current.title}</span>
			</nav>
			<p role="status" aria-live="off" class="flex items-center gap-2 text-xs text-muted">
				<span aria-hidden="true" class={['size-2 rounded-full', online ? 'bg-ok' : 'bg-warning']}
				></span>
				{online ? 'Connecté au gestionnaire' : 'Non connecté au gestionnaire'}
			</p>
		</header>

		{#if greeted}
			<StatusBanner {locked} {online} />
		{/if}

		<main
			id={MAIN_ID}
			tabindex="-1"
			class="mx-auto flex w-full max-w-[80rem] flex-1 flex-col gap-4 p-6 outline-none"
		>
			{#if greeted}
				<Notices
					notices={deck.notices}
					fallbackFocusId={MAIN_ID}
					ondismiss={(id) => deck.dismiss(id)}
				/>
				<!-- Disabled, not inert: the page stays in the accessibility tree and can be read. -->
				<fieldset
					bind:this={content}
					disabled={!usable}
					class="m-0 flex min-w-0 flex-col gap-4 border-0 p-0"
				>
					{@render children()}
				</fieldset>
			{:else}
				<div class="m-auto flex flex-col items-center gap-2 text-center">
					<p class="font-display text-xl font-semibold">Connexion au gestionnaire…</p>
					<p class="text-muted">
						Si rien ne se passe, vérifiez que le service hubergamof est lancé.
					</p>
				</div>
			{/if}
		</main>
	</div>
</div>
