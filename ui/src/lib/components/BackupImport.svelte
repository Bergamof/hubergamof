<!--
@component
Import of a backup in three steps: the file, what importing it would do (the sections are ticked
one by one, what would run on this computer is listed) and what it did, with the way back. The
file is read once and these bytes serve both the preview and the import: what is shown is what is
imported, even if the file changes on disk meanwhile. Before anything is written the manager
takes a snapshot of what is there; « Annuler l’import » puts it back.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import { ARCHIVE_TOO_LARGE, MAX_ARCHIVE_BYTES, type ManagerApi } from '#lib/api/manager-api.js';
	import type { BackupPreview, DeviceInfo, ImportReport } from '#lib/generated/index.js';
	import { Operation } from '#lib/deck/operation.svelte.js';
	import BackupPreviewList from './BackupPreviewList.svelte';
	import BackupReport from './BackupReport.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import ProblemAlert from './ProblemAlert.svelte';

	interface Props {
		api: ManagerApi;
		/** Name of each connected device and mouse, by serial. */
		names: ReadonlyMap<string, string>;
		/** The Stream Deck that are connected. */
		decks: readonly DeviceInfo[];
		/** The configuration changed (an import, an undo): snapshots may have been taken. */
		onchange?: () => void;
	}

	let { api, names, decks, onchange }: Props = $props();

	const uid = $props.id();

	/** Before « ? » and inside « », as French typography wants. */
	const NBSP = ' ';

	type Step = 'file' | 'preview' | 'report';

	const operation = new Operation();
	let step: Step = $state('file');
	let pickButton: HTMLButtonElement | undefined = $state();
	let input: HTMLInputElement | undefined = $state();
	let heading: HTMLHeadingElement | undefined = $state();
	let undoButton: HTMLButtonElement | undefined = $state();
	let undoneHeading: HTMLHeadingElement | undefined = $state();

	/** The bytes of the file chosen, read once. */
	let bytes: Blob | undefined;
	let fileName = $state('');
	let preview: BackupPreview | undefined = $state();
	let selected: string[] = $state([]);
	let receivers: Record<string, string> = $state({});
	let report: ImportReport | undefined = $state();
	let undone: ImportReport | undefined = $state();
	let confirmingUndo = $state(false);

	const importable = $derived(selected.length > 0);

	/** Moves the focus to the heading of the step that replaces the last one. */
	async function show(next: Step) {
		step = next;
		await tick();
		heading?.focus();
	}

	function choose() {
		if (operation.busy) return;
		operation.clear();
		input?.click();
	}

	async function picked(event: Event & { currentTarget: HTMLInputElement }) {
		const file = event.currentTarget.files?.[0];
		event.currentTarget.value = '';
		if (!file) return;
		// Refused before the whole file is read into memory.
		if (file.size > MAX_ARCHIVE_BYTES) {
			await operation.run(async () => ({ ...ARCHIVE_TOO_LARGE }), '');
			return;
		}
		const content = new Blob([await file.arrayBuffer()], { type: 'application/zip' });
		const ok = await operation.run(async () => {
			const result = await api.previewBackup(content);
			if (!result.ok) return result.problem;
			bytes = content;
			fileName = file.name;
			preview = result.value;
			// What would not change is left out: ticking it is the person's choice.
			selected = result.value.sections
				.filter((section) => section.change !== 'same')
				.map((section) => section.section);
			receivers = {};
			undone = undefined;
			return undefined;
		}, `Fichier lu : choisissez ce qui est importé.`);
		if (ok) await show('preview');
	}

	function startOver() {
		bytes = undefined;
		preview = undefined;
		report = undefined;
		undone = undefined;
		operation.clear();
		void show('file').then(() => pickButton?.focus());
	}

	async function importNow() {
		const content = bytes;
		if (operation.busy || !importable || !content) return;
		const devices = Object.fromEntries(
			Object.entries(receivers).filter(([serial]) => selected.includes(`devices/${serial}`))
		);
		const ok = await operation.run(async () => {
			const result = await api.importBackup(content, { sections: selected, devices });
			if (!result.ok) return result.problem;
			report = result.value;
			return undefined;
		}, 'Import terminé : le résultat de chaque section est affiché.');
		if (report) onchange?.();
		if (ok) await show('report');
	}

	async function undoImport() {
		const snapshot = report?.snapshot;
		confirmingUndo = false;
		if (!snapshot) return;
		await operation.run(async () => {
			const result = await api.restoreSnapshot(snapshot);
			if (!result.ok) return result.problem;
			undone = result.value;
			return undefined;
		}, 'Import annulé : l’état d’avant est remis.');
		if (undone) {
			onchange?.();
			// The button that opened the confirmation is gone: the result takes the focus.
			await tick();
			undoneHeading?.focus();
		}
	}
</script>

<section aria-labelledby="{uid}-title" class="flex flex-col gap-3 card">
	<h2 id="{uid}-title" class="text-lg font-semibold">Importer</h2>

	{#if step === 'file'}
		<h3 bind:this={heading} tabindex="-1" class="font-medium">1. Choisir le fichier</h3>
		<p class="text-sm text-muted">
			Une sauvegarde exportée par HuBergamof, ou l’ancien export d’un seul Stream Deck. Rien n’est
			écrit avant que vous ayez vu ce qu’elle contient.
		</p>
		<div>
			<button
				bind:this={pickButton}
				type="button"
				class="secondary-button"
				aria-disabled={operation.busy}
				onclick={choose}
			>
				{operation.busy ? 'Lecture…' : 'Choisir un fichier…'}
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
	{:else if step === 'preview' && preview}
		<h3 bind:this={heading} tabindex="-1" class="font-medium">2. Vérifier ce qui sera importé</h3>
		<p class="text-sm">Fichier&nbsp;: <span class="font-mono wrap-anywhere">{fileName}</span></p>
		<BackupPreviewList {preview} {names} {decks} headingLevel={4} bind:selected bind:receivers />
		<p class="text-sm text-muted">
			Avant d’écrire quoi que ce soit, HuBergamof garde un instantané de la configuration actuelle :
			vous pourrez annuler l’import. N’importez que des fichiers de confiance.
		</p>
		<div class="flex flex-wrap gap-2">
			<button
				type="button"
				class="primary-button"
				aria-disabled={operation.busy || !importable}
				onclick={importNow}
			>
				{#if operation.busy}
					Import…
				{:else if importable}
					Importer {selected.length} section{selected.length > 1 ? 's' : ''}
				{:else}
					Rien à importer
				{/if}
			</button>
			<button type="button" class="secondary-button" onclick={startOver}>
				Choisir un autre fichier
			</button>
		</div>
	{:else if step === 'report' && report}
		<h3 bind:this={heading} tabindex="-1" class="font-medium">3. Résultat</h3>
		<BackupReport {report} {names} />
		{#if undone}
			<h4 bind:this={undoneHeading} tabindex="-1" class="font-medium">Import annulé</h4>
			<BackupReport report={undone} {names} />
		{/if}
		<div class="flex flex-wrap gap-2">
			{#if !undone}
				<button
					bind:this={undoButton}
					type="button"
					class="secondary-button"
					aria-disabled={operation.busy}
					onclick={() => {
						if (!operation.busy) confirmingUndo = true;
					}}
				>
					Annuler l’import
				</button>
			{/if}
			<button type="button" class="secondary-button" onclick={startOver}>
				Importer un autre fichier
			</button>
		</div>
	{/if}

	<p role="status" class="text-sm text-muted">{operation.status}</p>
	{#if operation.problem}
		<ProblemAlert problem={operation.problem} lead="Import non effectué." />
	{/if}
</section>

<ConfirmDialog
	open={confirmingUndo}
	title={`Remettre l’état d’avant l’import${NBSP}?`}
	confirmLabel="Remettre l’état d’avant"
	onconfirm={undoImport}
	oncancel={() => (confirmingUndo = false)}
	onclosed={() => undoButton?.focus()}
>
	<p>
		L’état d’avant l’import est remis, section par section. Ce qui a été créé depuis (à la main, ou
		par un autre import) est remplacé aussi : un nouvel instantané est pris, pour pouvoir revenir.
	</p>
</ConfirmDialog>
