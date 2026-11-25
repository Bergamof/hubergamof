<!--
@component
The devices that are connected, or what to do when none is. They are looked for all the time: nothing to
press to search.
-->
<script lang="ts">
	import { getManager } from '#lib/deck/manager-context.js';
	import { deviceHref, navDevices, type NavDevice } from '#lib/deck/nav-model.js';
	import EmptyState from './EmptyState.svelte';
	import Icon, { type IconName } from './Icon.svelte';
	import PageHeader from './PageHeader.svelte';

	const { deck } = getManager();

	const devices = $derived(navDevices(deck.devices, deck.mice.mice, deck.headsets.headsets));

	const ICON: Record<NavDevice['kind'], IconName> = {
		deck: 'deck',
		mouse: 'mouse',
		headset: 'headset'
	};
	const KIND: Record<NavDevice['kind'], string> = {
		deck: 'Stream Deck',
		mouse: 'Souris',
		headset: 'Casque'
	};
</script>

<PageHeader
	title="Périphériques"
	description={devices.length === 0
		? undefined
		: `${devices.length} connecté${devices.length > 1 ? 's' : ''}`}
/>

{#if devices.length === 0}
	<EmptyState title="Aucun périphérique connecté">
		<p>Ils apparaissent ici dès qu’ils sont branchés ou allumés.</p>
		<ul class="mt-2 flex list-disc flex-col gap-1.5 pl-5 text-left">
			<li>
				<strong>Stream Deck :</strong> branchez-le en USB (la règle udev doit être installée).
			</li>
			<li><strong>Souris MX :</strong> allumez-la et branchez le récepteur Logi Bolt.</li>
			<li><strong>Casque Sony :</strong> appairez-le dans les réglages Bluetooth du système.</li>
		</ul>
		<p class="mt-2 text-xs">Recherche en continu, toutes les 5 secondes.</p>
	</EmptyState>
{:else}
	<ul class="grid grid-cols-[repeat(auto-fill,minmax(14rem,1fr))] gap-3">
		{#each devices as device (device.kind + device.id)}
			<li>
				<a
					href={deviceHref(device)}
					data-sveltekit-preload-data="off"
					class="flex flex-col gap-1 card text-fg no-underline focus-ring hover:border-edge"
				>
					<span class="flex items-center gap-2 font-semibold">
						<Icon name={ICON[device.kind]} />{device.label}
					</span>
					<span class="text-muted">{KIND[device.kind]}</span>
				</a>
			</li>
		{/each}
	</ul>
{/if}
