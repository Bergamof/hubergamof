<!--
@component
Export of the configuration of a device to a zip (config.json and its images), and import of
such an archive in place of the current configuration, after confirmation.
-->
<script lang="ts">
	import type { DeviceInfo } from '#lib/generated/index.js';
	import { type ManagerApi, exportFileName, exportUrl } from '#lib/api/manager-api.js';
	import { Operation } from '#lib/deck/operation.svelte.js';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import ProblemAlert from './ProblemAlert.svelte';

	interface Props {
		device: DeviceInfo;
		api: ManagerApi;
	}

	let { device, api }: Props = $props();

	/** Before « ? » and inside « », as French typography wants. */
	const NBSP = '\u00a0';

	const operation = new Operation();
	let importButton: HTMLButtonElement | undefined = $state();
	let input: HTMLInputElement | undefined = $state();
	let chosen: File | undefined = $state();

	/** Names the download on the day it is made (the page may stay open for days). */
	function nameDownload(event: MouseEvent & { currentTarget: HTMLAnchorElement }) {
		operation.clear();
		event.currentTarget.download = exportFileName(device.serial, new Date());
	}

	function choose() {
		if (operation.busy) return;
		operation.clear();
		input?.click();
	}

	function picked(event: Event & { currentTarget: HTMLInputElement }) {
		chosen = event.currentTarget.files?.[0];
		event.currentTarget.value = '';
	}

	/**
	 * Back to « Importer… » once the confirmation closes: it opened from the file chooser,
	 * so the dialog cannot always tell where the focus was.
	 */
	function closeConfirmation() {
		chosen = undefined;
		importButton?.focus();
	}

	async function confirmImport() {
		const file = chosen;
		closeConfirmation();
		if (!file) return;
		await operation.run(async () => {
			const result = await api.importArchive(device.serial, file);
			return result.ok ? undefined : result.problem;
		}, `Configuration importée depuis «${NBSP}${file.name}${NBSP}».`);
	}
</script>

<section class="flex flex-col gap-2" aria-label="Sauvegarde de la configuration">
	<div class="flex flex-wrap gap-2">
		<!-- A route of the manager, not of the UI: `external` keeps the client router out. -->
		<a
			class="secondary-button"
			href={exportUrl(device.serial)}
			rel="external"
			download
			onclick={nameDownload}
		>
			Exporter la configuration
		</a>
		<button
			bind:this={importButton}
			type="button"
			class="secondary-button"
			aria-disabled={operation.busy}
			onclick={choose}
		>
			{operation.busy ? 'Import…' : 'Importer une configuration…'}
		</button>
	</div>
	<input
		bind:this={input}
		type="file"
		accept=".zip,application/zip"
		class="hidden"
		tabindex="-1"
		aria-hidden="true"
		onchange={picked}
	/>
	<p role="status" class="text-sm text-zinc-700 dark:text-zinc-300">{operation.status}</p>
	{#if operation.problem}
		<ProblemAlert problem={operation.problem} lead="Configuration non importée." />
	{/if}
</section>

<ConfirmDialog
	open={chosen !== undefined}
	title={`Remplacer la configuration${NBSP}?`}
	confirmLabel="Remplacer"
	onconfirm={confirmImport}
	oncancel={closeConfirmation}
>
	<p>
		Les touches, molettes et réglages de {device.displayName} seront remplacés par ceux de « {chosen?.name}
		». Exportez d’abord la configuration actuelle pour pouvoir y revenir.
	</p>
	<p class="mt-2 font-medium">
		N’importez que des fichiers de confiance : une configuration peut lancer des commandes, par
		exemple des projets Docker.
	</p>
</ConfirmDialog>
