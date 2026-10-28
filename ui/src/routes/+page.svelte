<script lang="ts">
	import { ManagerConnection, managerSocketUrl } from '#lib/connection/index.js';
	import type { DeviceInfo } from '#lib/generated/index.js';

	let connection = $state<ManagerConnection>();
	let devices = $state<DeviceInfo[]>([]);

	const isOnline = $derived(connection?.status === 'open');

	$effect(() => {
		const managerConnection = new ManagerConnection({ url: managerSocketUrl(location) });
		const unsubscribe = managerConnection.onMessage((message) => {
			if (message.type === 'devices') devices = message.devices;
		});
		managerConnection.connect();
		connection = managerConnection;

		return () => {
			unsubscribe();
			managerConnection.disconnect();
		};
	});
</script>

<svelte:head>
	<title>Bergadeck</title>
</svelte:head>

<div class="mx-auto flex min-h-screen max-w-5xl flex-col gap-6 p-6">
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

	<main class="flex flex-1 flex-col">
		<h2 class="sr-only">Appareils</h2>
		{#if !isOnline}
			<p class="m-auto text-zinc-600 dark:text-zinc-400">Connexion au gestionnaire…</p>
		{:else if devices.length === 0}
			<p class="m-auto text-zinc-600 dark:text-zinc-400">Aucun appareil connecté</p>
		{:else}
			<ul class="flex flex-col gap-2">
				{#each devices as device (device.serial)}
					<li>{device.displayName}</li>
				{/each}
			</ul>
		{/if}
	</main>
</div>
