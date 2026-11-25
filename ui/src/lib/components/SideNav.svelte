<!--
@component
Main navigation: the home page, then the devices that are connected (they come and go, the group stays), the
machine and the application. The page shown is marked with `aria-current`.
-->
<script lang="ts">
	import { resolve } from '$app/paths';
	import type { ResolvedPathname } from '$app/types';
	import { PATHS, deviceHref, type NavDevice } from '#lib/deck/nav-model.js';
	import BrandWordmark from './BrandWordmark.svelte';
	import Icon, { type IconName } from './Icon.svelte';

	interface Props {
		pathname: string;
		devices: readonly NavDevice[];
		/** Serial of the Stream Deck and of the mouse the pages follow. */
		selectedDeck: string | undefined;
		selectedMouse: string | undefined;
		/** Battery of the mouse followed, once read. */
		mouseBattery: number | undefined;
		updateAvailable: boolean;
		/** Called after a link is followed (closes a drawer, for instance). */
		onnavigate?: () => void;
	}

	let {
		pathname,
		devices,
		selectedDeck,
		selectedMouse,
		mouseBattery,
		updateAvailable,
		onnavigate
	}: Props = $props();

	const DEVICE_ICON: Record<NavDevice['kind'], IconName> = {
		deck: 'deck',
		mouse: 'mouse',
		headset: 'headset'
	};

	const machine = [
		{ href: PATHS.aliases, label: 'Alias du shell', icon: 'terminal' },
		{ href: PATHS.git, label: 'Git', icon: 'git' },
		{ href: PATHS.ssh, label: 'SSH', icon: 'server' }
	] as const;

	const application = [
		{ href: PATHS.modules, label: 'Modules', icon: 'modules' },
		{ href: PATHS.radial, label: 'Menu circulaire', icon: 'radial' },
		{ href: PATHS.updates, label: 'Mises à jour', icon: 'refresh' },
		{ href: PATHS.backup, label: 'Sauvegarde', icon: 'download' }
	] as const;

	const clean = (path: string) => (path.length > 1 ? path.replace(/\/+$/, '') : path);
	const here = $derived(clean(pathname));

	/** A device is current when its page is shown and the page follows it. */
	function deviceCurrent(device: NavDevice): boolean {
		if (here !== device.path) return false;
		if (device.kind === 'deck') return selectedDeck === device.id;
		if (device.kind === 'mouse') return selectedMouse === device.id;
		return true;
	}

	const LINK =
		'relative flex min-h-8 items-center gap-2 rounded-lg px-2 text-fg no-underline hover:bg-sunken focus-ring aria-[current=page]:bg-tint aria-[current=page]:font-semibold aria-[current=page]:before:absolute aria-[current=page]:before:inset-y-1.5 aria-[current=page]:before:-left-2 aria-[current=page]:before:w-[3px] aria-[current=page]:before:rounded-r aria-[current=page]:before:bg-accent';
	const GROUP_TITLE = 'px-2 pb-1 text-xs font-semibold tracking-wider text-muted uppercase';
</script>

{#snippet item(
	href: ResolvedPathname,
	label: string,
	icon: IconName,
	current: boolean,
	trailing?: string
)}
	<a
		{href}
		class={LINK}
		aria-current={current ? 'page' : undefined}
		data-sveltekit-preload-data="off"
		onclick={onnavigate}
	>
		<Icon name={icon} />
		<span class="min-w-0 flex-1 truncate">{label}</span>
		{#if trailing}
			<span class="text-xs text-muted tabular-nums">{trailing}</span>
		{/if}
	</a>
{/snippet}

<nav aria-label="Navigation principale" class="flex flex-col gap-4">
	<div class="px-2 pt-1">
		<a href={resolve(PATHS.home)} class="inline-flex focus-ring" aria-label="HuBergamof, accueil">
			<BrandWordmark class="h-7 w-auto text-fg" />
		</a>
	</div>

	<ul class="flex flex-col gap-0.5">
		<li>{@render item(resolve(PATHS.home), 'Accueil', 'home', here === PATHS.home)}</li>
	</ul>

	<section aria-labelledby="nav-devices">
		<h2 id="nav-devices" class={GROUP_TITLE}>Périphériques</h2>
		<ul class="flex flex-col gap-0.5">
			{#each devices as device (device.kind + device.id)}
				<li>
					{@render item(
						deviceHref(device),
						device.label,
						DEVICE_ICON[device.kind],
						deviceCurrent(device),
						device.kind === 'mouse' && device.id === selectedMouse && mouseBattery !== undefined
							? `${mouseBattery} %`
							: undefined
					)}
				</li>
			{:else}
				<li class="px-2 py-1.5 text-muted italic">
					<a href={resolve(PATHS.devices)} class="link" onclick={onnavigate}>Aucun connecté</a>
				</li>
			{/each}
		</ul>
	</section>

	<section aria-labelledby="nav-machine">
		<h2 id="nav-machine" class={GROUP_TITLE}>Poste</h2>
		<ul class="flex flex-col gap-0.5">
			{#each machine as entry (entry.href)}
				<li>{@render item(resolve(entry.href), entry.label, entry.icon, here === entry.href)}</li>
			{/each}
		</ul>
	</section>

	<section aria-labelledby="nav-application">
		<h2 id="nav-application" class={GROUP_TITLE}>Application</h2>
		<ul class="flex flex-col gap-0.5">
			{#each application as entry (entry.href)}
				<li>
					{@render item(
						resolve(entry.href),
						entry.label,
						entry.icon,
						here === entry.href,
						entry.href === PATHS.updates && updateAvailable ? 'Disponible' : undefined
					)}
				</li>
			{/each}
		</ul>
	</section>
</nav>
