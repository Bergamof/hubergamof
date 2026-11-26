<!--
@component
Version of the manager, result of its check for a newer release and, when the manager is installed
from a system package, the installation of that release (at once, or by itself as it is found).
The release notes open in a new tab.
-->
<script lang="ts">
	import type { UpdateSettings, UpdateStatus } from '#lib/generated/index.js';
	import { checkedLabel } from '#lib/deck/update-labels.js';

	interface Props {
		/** Running version, unknown until the manager greets the interface. */
		version: string | undefined;
		status: UpdateStatus;
		/** How updates are handled, unknown until the manager says. */
		settings: UpdateSettings | undefined;
		oncheck: () => void;
		oninstall: () => void;
		onautoinstall: (enabled: boolean) => void;
		oncheckdaily: (enabled: boolean) => void;
	}

	let { version, status, settings, oncheck, oninstall, onautoinstall, oncheckdaily }: Props =
		$props();

	const uid = $props.id();

	/** How long the manager has to confirm a choice before the page says it did not. */
	const CONFIRMATION_DELAY_MS = 5000;

	type Choice = 'checkDaily' | 'autoInstall';
	/** The value asked for each switch and not confirmed yet. */
	let asked = $state<Partial<Record<Choice, boolean>>>({});
	let unconfirmed = $state<Choice | undefined>(undefined);

	const pending = $derived(
		settings !== undefined &&
			(['checkDaily', 'autoInstall'] as const).some(
				(choice) => asked[choice] !== undefined && asked[choice] !== settings[choice]
			)
	);

	/**
	 * Asks for the choice and keeps showing the confirmed value: only the manager's `updateSettings`
	 * changes it. Meanwhile the switch stays (`aria-disabled`, so the focus is kept) and a status
	 * line says it is being saved, then that the manager did not answer.
	 */
	function choose(choice: Choice, input: HTMLInputElement, ask: (enabled: boolean) => void): void {
		const current = settings?.[choice];
		if (pending || current === undefined) {
			input.checked = current ?? false;
			return;
		}
		asked[choice] = input.checked;
		unconfirmed = undefined;
		ask(input.checked);
		input.checked = current;
		setTimeout(() => {
			if (settings && asked[choice] !== undefined && asked[choice] !== settings[choice]) {
				unconfirmed = choice;
				asked[choice] = undefined;
			}
		}, CONFIRMATION_DELAY_MS);
	}

	/** When the last check ran, known once one has found something (or nothing). */
	const checkedAt = $derived(
		status.type === 'available' || status.type === 'upToDate' ? status.checkedAt : undefined
	);

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
	aria-labelledby="{uid}-state"
	class={['flex flex-col gap-3 card p-4 text-sm', status.type === 'available' && 'border-ring']}
>
	<h2 id="{uid}-state" class="font-display text-base font-semibold">Version</h2>
	<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
		{#if status.type === 'available' || status.type === 'upToDate'}
			<span class="rounded-full border border-line px-2 py-0.5 text-sm font-medium">
				{status.type === 'available' ? 'Nouveau' : 'À jour'}
			</span>
		{/if}
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
	{#if checkedAt !== undefined}
		<p role="status" class="text-muted">Vérifiée {checkedLabel(checkedAt)}.</p>
	{/if}
</section>

{#if settings}
	<section aria-labelledby="{uid}-settings" class="mt-4 flex flex-col gap-3 card p-4 text-sm">
		<h2 id="{uid}-settings" class="font-display text-base font-semibold">Réglages</h2>
		<!-- Not the shared `Switch`: these must show the value the manager confirmed, never the one clicked. -->
		<label class="flex items-center gap-2.5">
			<input
				type="checkbox"
				role="switch"
				class="switch"
				aria-disabled={pending}
				aria-describedby="{uid}-daily-help"
				checked={settings.checkDaily}
				onchange={(event) => choose('checkDaily', event.currentTarget, oncheckdaily)}
			/>
			Vérifier une fois par jour
		</label>
		<p id="{uid}-daily-help" class="text-muted">
			Interroge GitHub une fois par jour, sans rien envoyer de vous. Désactivée, la recherche ne se
			fait que sur demande.
		</p>
		{#if canInstall}
			<label class="flex items-center gap-2.5">
				<input
					type="checkbox"
					role="switch"
					class="switch"
					aria-disabled={pending}
					checked={settings.autoInstall}
					onchange={(event) => choose('autoInstall', event.currentTarget, onautoinstall)}
				/>
				Installer automatiquement les mises à jour
			</label>
		{:else}
			<p class="text-muted">
				L’installation automatique n’est pas disponible ici : elle se fait à la main.
			</p>
		{/if}
		<p role="status" class="text-muted empty:hidden">
			{#if pending}Enregistrement…{:else if unconfirmed}Le gestionnaire n’a pas confirmé le réglage
				: réessayez.{/if}
		</p>
	</section>
{/if}
