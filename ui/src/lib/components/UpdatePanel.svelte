<!--
@component
Version of the manager and result of its check for a newer release, with a button to check now.
The release notes open in a new tab: the update itself is installed from the system's packages.
-->
<script lang="ts">
	import type { UpdateStatus } from '#lib/generated/index.js';

	interface Props {
		/** Running version, unknown until the manager greets the interface. */
		version: string | undefined;
		status: UpdateStatus;
		oncheck: () => void;
	}

	let { version, status, oncheck }: Props = $props();

	// `aria-disabled`, not `disabled`: the button keeps the focus while the check runs.
	const checking = $derived(status.type === 'checking');
</script>

<section
	aria-label="Mises à jour"
	class={[
		'flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border p-3 text-sm',
		status.type === 'available' ? 'border-ring bg-surface' : 'border-line'
	]}
>
	<p role="status" class="flex-1">
		{#if status.type === 'available'}
			<strong class="font-semibold">Version {status.version} disponible.</strong>
			{#if version}Vous utilisez la version {version}.{/if}
			<!-- External https link (checked by the manager), not a route of this app. -->
			<!-- eslint-disable svelte/no-navigation-without-resolve -->
			<a
				href={status.notesUrl}
				target="_blank"
				rel="noopener noreferrer"
				class="underline focus-ring"
			>
				Notes de version<span class="sr-only"> (s’ouvre dans un nouvel onglet)</span>
			</a>
			<!-- eslint-enable svelte/no-navigation-without-resolve -->
		{:else if status.type === 'checking'}
			Recherche de mises à jour…
		{:else if status.type === 'upToDate'}
			HuBergamof est à jour{version ? ` (version ${version})` : ''}.
		{:else if status.type === 'checkFailed'}
			La recherche de mises à jour a échoué. Détail&nbsp;: <span lang="en">{status.message}</span>
		{:else}
			{version ? `Version ${version}.` : 'Version inconnue.'} Aucune recherche de mise à jour effectuée.
		{/if}
	</p>
	<button
		type="button"
		class="secondary-button"
		aria-disabled={checking}
		onclick={() => {
			if (!checking) oncheck();
		}}
	>
		Rechercher des mises à jour
	</button>
</section>
