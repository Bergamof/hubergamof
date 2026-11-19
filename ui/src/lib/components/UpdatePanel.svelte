<!--
@component
Version of the manager, result of its check for a newer release and, when the manager is installed
from a system package, the installation of that release (at once, or by itself as it is found).
The release notes open in a new tab.
-->
<script lang="ts">
	import type { UpdateSettings, UpdateStatus } from '#lib/generated/index.js';

	interface Props {
		/** Running version, unknown until the manager greets the interface. */
		version: string | undefined;
		status: UpdateStatus;
		/** How updates are handled, unknown until the manager says. */
		settings: UpdateSettings | undefined;
		oncheck: () => void;
		oninstall: () => void;
		onautoinstall: (enabled: boolean) => void;
	}

	let { version, status, settings, oncheck, oninstall, onautoinstall }: Props = $props();

	const uid = $props.id();

	/** Defence in depth: the manager already checks the link, the page still only follows https. */
	const notesLink = $derived(
		status.type === 'available' && status.notesUrl.startsWith('https://')
			? status.notesUrl
			: undefined
	);
	/** Something is under way: neither button does anything meanwhile. */
	const busy = $derived(
		status.type === 'checking' || status.type === 'downloading' || status.type === 'installing'
	);
	const canInstall = $derived(settings !== undefined && settings.installSupport !== 'unpackaged');
	/** The button stays while it works (same element, `aria-disabled`): the focus is not lost. */
	const installable = $derived(
		canInstall &&
			(status.type === 'available' ||
				status.type === 'installFailed' ||
				status.type === 'downloading' ||
				status.type === 'installing')
	);
</script>

<section
	aria-label="Mises à jour"
	class={[
		'flex flex-col gap-3 rounded-xl border p-3 text-sm',
		status.type === 'available' ? 'border-ring bg-surface' : 'border-line'
	]}
>
	<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
		<p role="status" class="flex-1 wrap-anywhere">
			{#if status.type === 'available'}
				<strong class="font-semibold">Version {status.version} disponible.</strong>
				{#if version}Vous utilisez la version {version}.{/if}
				{#if notesLink}
					<!-- External https link (checked by the manager), not a route of this app. -->
					<!-- eslint-disable svelte/no-navigation-without-resolve -->
					<a
						href={notesLink}
						target="_blank"
						rel="noopener noreferrer"
						class="underline focus-ring"
					>
						Notes de version<span class="sr-only"> (s’ouvre dans un nouvel onglet)</span>
					</a>
					<!-- eslint-enable svelte/no-navigation-without-resolve -->
				{/if}
				{#if settings && !canInstall}
					Cette installation ne se met pas à jour seule : installez le paquet de la nouvelle
					version.
				{/if}
			{:else if status.type === 'checking'}
				Recherche de mises à jour…
			{:else if status.type === 'downloading'}
				Téléchargement de la version {status.version}…
			{:else if status.type === 'installing'}
				Installation de la version {status.version}…
			{:else if status.type === 'installed'}
				Version {status.version} installée : le service redémarre.
			{:else if status.type === 'installFailed'}
				L’installation de la version {status.version} a échoué. Détail&nbsp;:
				<span lang="en">{status.message}</span>
			{:else if status.type === 'upToDate'}
				HuBergamof est à jour{version ? ` (version ${version})` : ''}.
			{:else if status.type === 'checkFailed'}
				La recherche de mises à jour a échoué. Détail&nbsp;: <span lang="en">{status.message}</span>
			{:else}
				{version ? `Version ${version}.` : 'Version inconnue.'} Aucune recherche de mise à jour effectuée.
			{/if}
		</p>
		<div class="flex flex-wrap gap-2">
			{#if installable}
				<!-- `aria-disabled`, not `disabled`: the button keeps the focus while it works. -->
				<button
					type="button"
					class="primary-button"
					aria-disabled={busy}
					onclick={() => {
						if (!busy) oninstall();
					}}
				>
					{status.type === 'installFailed' ? 'Réessayer' : 'Mettre à jour maintenant'}
				</button>
			{/if}
			<button
				type="button"
				class="secondary-button"
				aria-disabled={busy}
				onclick={() => {
					if (!busy) oncheck();
				}}
			>
				Rechercher des mises à jour
			</button>
		</div>
	</div>
	{#if canInstall && settings}
		<label class="flex min-h-9 items-center gap-2" for="{uid}-auto">
			<input
				id="{uid}-auto"
				type="checkbox"
				class="size-5 accent-ring focus-ring"
				checked={settings.autoInstall}
				onchange={(event) => {
					onautoinstall(event.currentTarget.checked);
					// Shown as the manager confirms it (`updateSettings`), never as merely asked.
					event.currentTarget.checked = settings.autoInstall;
				}}
			/>
			Installer automatiquement les mises à jour
		</label>
	{/if}
</section>
