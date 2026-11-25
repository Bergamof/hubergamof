<!--
@component
The home page: what is connected, what the modules of the application show now, whether an update is waiting,
and a few ways into the rest. Only what exists is shown, and nothing here opens the link of the headphones:
it would cut the Sony application: the battery of the headphones is read on request, by a button.
-->
<script lang="ts">
	import { resolve } from '$app/paths';
	import { getManager } from '#lib/deck/manager-context.js';
	import { PATHS, deviceHref, navDevices, type NavDevice } from '#lib/deck/nav-model.js';
	import { moduleName } from '#lib/deck/display.js';
	import { availableActions, groupActions, isEnding } from '#lib/deck/module-actions.js';
	import BatteryBadge from './BatteryBadge.svelte';
	import Icon, { type IconName } from './Icon.svelte';
	import PageHeader from './PageHeader.svelte';

	const { deck } = getManager();

	const devices = $derived(
		navDevices(deck.devices, deck.mice.mice, deck.headsets.headsets, (address) =>
			deck.headsets.batteryOf(address)
		)
	);
	const modules = $derived(deck.application.modules);

	const DEVICE_ICON: Record<NavDevice['kind'], IconName> = {
		deck: 'deck',
		mouse: 'mouse',
		headset: 'headset'
	};
	const MODULE_ICON: Record<string, IconName> = {
		pomodoro: 'timer',
		audio: 'volume',
		teams: 'chat',
		intellij: 'code',
		claude: 'spark'
	};

	/** Opens the link to the headphones to read their battery, and says what became of it. */
	let headsetNotice = $state('');
	let wasReading = false;
	$effect(() => {
		const reading = deck.headsets.reading;
		if (reading) {
			headsetNotice = 'Lecture de la batterie du casque…';
		} else if (wasReading) {
			const battery = deck.headsets.headsets[0]
				? deck.headsets.batteryOf(deck.headsets.headsets[0].address)
				: undefined;
			headsetNotice = battery
				? `Batterie du casque : ${battery.percent} %.`
				: 'Batterie du casque non lue : la liaison n’a pas pu s’ouvrir.';
		}
		wasReading = reading;
	});

	function readBattery() {
		if (!deck.headsets.reading) deck.headsets.readBattery();
	}

	/** The lines a module says: the step, then the detail. */
	const linesOf = (status: string | null | undefined): string[] =>
		(status ?? '').split('\n').filter((line) => line !== '');

	/** Runs the first action of a module; the connection being lost, nothing was sent, which is said. */
	function run(kind: string, action: string) {
		if (deck.application.run(kind, action)) return;
		deck.report({
			text: 'Action non envoyée : la connexion au gestionnaire est perdue.',
			detail: null
		});
	}

	const update = $derived(deck.updateStatus);
	const SHORTCUTS = [
		{ href: PATHS.radial, label: 'Régler le menu circulaire' },
		{ href: PATHS.ssh, label: 'Gérer les serveurs SSH' },
		{ href: PATHS.backup, label: 'Sauvegarder la configuration' }
	];
</script>

<PageHeader
	title="Accueil"
	description="{devices.length} périphérique{devices.length > 1
		? 's'
		: ''} connecté{devices.length > 1 ? 's' : ''}, {modules.length} module{modules.length > 1
		? 's'
		: ''} dans l’application"
/>

<section class="flex flex-col gap-3 card" aria-labelledby="home-devices">
	<div class="flex items-center justify-between">
		<h2 id="home-devices" class="text-lg font-semibold">Périphériques</h2>
		<a href={resolve(PATHS.devices)} class="inline-flex items-center gap-1 link">
			Tout voir<span class="sr-only"> les périphériques</span>
			<Icon name="chevronRight" class="size-3.5" />
		</a>
	</div>
	{#if devices.length === 0}
		<p class="text-muted">
			Aucun périphérique connecté. Ils apparaissent ici dès qu’ils sont branchés ou allumés.
		</p>
	{:else}
		<ul class="grid grid-cols-[repeat(auto-fill,minmax(13rem,1fr))] gap-3">
			{#each devices as device (device.kind + device.id)}
				<li class="flex flex-col gap-2 rounded-lg border border-line p-3">
					<span class="flex items-center gap-2 font-semibold">
						<Icon name={DEVICE_ICON[device.kind]} />{device.label}
					</span>
					<span class="text-sm">
						{#if device.kind === 'deck'}
							<span class="text-muted">Connecté en USB</span>
						{:else}
							<BatteryBadge
								battery={device.battery}
								unknown={device.kind === 'headset' ? 'non lue' : 'inconnue'}
								labelled
							/>
						{/if}
					</span>
					<span class="mt-auto flex flex-wrap gap-2">
						{#if device.kind === 'headset'}
							<!-- `aria-disabled`, not `disabled`: the button keeps the focus while the link opens. -->
							<button
								type="button"
								class="secondary-button"
								aria-disabled={deck.headsets.reading}
								aria-describedby="home-headset-reading"
								onclick={readBattery}
							>
								{#if deck.headsets.reading}
									Lecture…
								{:else}
									{device.battery ? 'Relire' : 'Lire'} la batterie<span class="sr-only">
										de {device.label}</span
									>
								{/if}
							</button>
						{/if}
						<a
							href={deviceHref(device)}
							data-sveltekit-preload-data="off"
							class="secondary-button no-underline"
						>
							Configurer<span class="sr-only"> {device.label}</span>
						</a>
					</span>
					{#if device.kind === 'headset'}
						<span class="text-xs text-muted">
							Lire la batterie ouvre la liaison avec le casque : l’application Sony se coupe jusqu’à
							ce qu’il soit libéré.
						</span>
					{/if}
				</li>
			{/each}
		</ul>
		<p id="home-headset-reading" role="status" class="sr-only">{headsetNotice}</p>
	{/if}
</section>

<div class="grid grid-cols-[minmax(0,2fr)_minmax(16rem,1fr)] gap-4">
	<section class="flex flex-col gap-3 card" aria-labelledby="home-modules">
		<div class="flex items-center justify-between">
			<h2 id="home-modules" class="text-lg font-semibold">Modules</h2>
			<a href={resolve(PATHS.modules)} class="inline-flex items-center gap-1 link">
				Réglages<span class="sr-only"> des modules</span>
				<Icon name="chevronRight" class="size-3.5" />
			</a>
		</div>
		{#if modules.length === 0}
			<p class="text-muted">Aucun module dans l’application.</p>
		{:else}
			<ul class="grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-3">
				{#each modules as view (view.kind)}
					{@const lines = linesOf(view.status)}
					{@const action = groupActions(
						view.kind,
						availableActions(deck.moduleOf(view.kind), view)
					)[0]?.actions.find((candidate) => !isEnding(candidate))}
					<li class="flex min-h-24 flex-col gap-1 rounded-lg border border-line p-3">
						<span class="flex items-center gap-1.5 font-medium text-muted">
							<Icon name={MODULE_ICON[view.kind] ?? 'modules'} />
							{moduleName(view.kind, deck.moduleOf(view.kind))}
						</span>
						{#if lines.length > 0}
							<span class="font-display text-lg font-semibold">{lines[0]}</span>
							{#if lines[1]}<span class="text-muted">{lines[1]}</span>{/if}
						{:else}
							<span class="text-muted">En attente</span>
						{/if}
						{#if action}
							<button
								type="button"
								class="mt-auto secondary-button self-start"
								onclick={() => run(view.kind, action.id)}
							>
								{action.name}<span class="sr-only">
									de {moduleName(view.kind, deck.moduleOf(view.kind))}</span
								>
							</button>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</section>

	<div class="flex flex-col gap-4">
		<section class="flex flex-col gap-2 card" aria-labelledby="home-updates">
			<h2 id="home-updates" class="text-lg font-semibold">Mises à jour</h2>
			<p>
				{#if update.type === 'available'}
					<strong>Version {update.version} disponible</strong>
					<span class="block text-muted">Version installée : {deck.managerVersion}.</span>
				{:else if update.type === 'upToDate'}
					<strong>Version {deck.managerVersion}</strong>
					<span class="block text-muted">À jour.</span>
				{:else}
					<strong>Version {deck.managerVersion}</strong>
				{/if}
			</p>
			<a href={resolve(PATHS.updates)} class="secondary-button self-start no-underline">
				{update.type === 'available' ? 'Mettre à jour' : 'Voir les mises à jour'}
			</a>
		</section>

		<section class="card" aria-labelledby="home-links">
			<h2 id="home-links" class="mb-1 text-lg font-semibold">Raccourcis</h2>
			<ul>
				{#each SHORTCUTS as shortcut (shortcut.href)}
					<li class="border-t border-line first:border-t-0">
						<a
							href={resolve(shortcut.href)}
							class="flex items-center justify-between py-2 text-fg no-underline focus-ring hover:text-accent"
						>
							{shortcut.label}<Icon name="chevronRight" class="size-3.5" />
						</a>
					</li>
				{/each}
			</ul>
		</section>
	</div>
</div>
