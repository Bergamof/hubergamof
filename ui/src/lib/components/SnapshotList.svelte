<!--
@component
The automatic snapshots the manager takes before every import and every restore (the latest few
are kept). Restoring one shows first what it would do, then puts it back, which is a snapshot-taking
write as well: a restore can be undone.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import type { ManagerApi } from '#lib/api/manager-api.js';
	import type { BackupPreview, ImportReport, SnapshotInfo } from '#lib/generated/index.js';
	import { Operation } from '#lib/deck/operation.svelte.js';
	import { sectionLabel, snapshotDate } from '#lib/deck/backup-labels.js';
	import BackupPreviewList from './BackupPreviewList.svelte';
	import BackupReport from './BackupReport.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import ProblemAlert from './ProblemAlert.svelte';

	interface Props {
		api: ManagerApi;
		/** Name of each connected device and mouse, by serial. */
		names: ReadonlyMap<string, string>;
		/** Changes whenever the list may have changed: it is read again. */
		version: number;
		/** The configuration changed (a restore): snapshots may have been taken. */
		onchange?: () => void;
	}

	let { api, names, version, onchange }: Props = $props();

	const uid = $props.id();

	/** Before « ? » as French typography wants. */
	const NBSP = ' ';

	const operation = new Operation();
	let snapshots: SnapshotInfo[] | undefined = $state();
	let pending: { snapshot: SnapshotInfo; preview: BackupPreview } | undefined = $state();
	let report: ImportReport | undefined = $state();
	let opener: HTMLElement | undefined;
	let heading: HTMLHeadingElement | undefined = $state();
	/** The preview of a snapshot is being read. */
	let reading = $state(false);

	$effect(() => {
		void version;
		void load();
	});

	async function load() {
		const result = await api.listSnapshots();
		if (result.ok) snapshots = result.value;
		else operation.problem = result.problem;
	}

	async function ask(snapshot: SnapshotInfo, button: HTMLElement) {
		if (operation.busy) return;
		opener = button;
		report = undefined;
		reading = true;
		await operation.run(async () => {
			const result = await api.previewSnapshot(snapshot.id);
			if (!result.ok) return result.problem;
			pending = { snapshot, preview: result.value };
			return undefined;
		}, '');
		reading = false;
	}

	async function restore() {
		const target = pending?.snapshot;
		pending = undefined;
		if (!target) return;
		await operation.run(async () => {
			const result = await api.restoreSnapshot(target.id);
			if (!result.ok) return result.problem;
			report = result.value;
			return undefined;
		}, 'Sauvegarde automatique restaurée : le résultat de chaque section est affiché.');
		if (report) {
			onchange?.();
			// The list is read again and the button that opened the dialog may be gone.
			await tick();
			heading?.focus();
		}
	}
</script>

<section aria-labelledby="{uid}-title" class="flex flex-col gap-3 card">
	<h2 bind:this={heading} id="{uid}-title" tabindex="-1" class="text-lg font-semibold">
		Sauvegardes automatiques
	</h2>
	<p class="text-sm text-muted">
		Avant chaque import, HuBergamof garde l’état de la configuration. Les 3 dernières sont
		conservées ; restaurer l’une d’elles en crée une nouvelle, donc se défait aussi.
	</p>
	{#if snapshots === undefined}
		<p class="text-sm text-muted">Chargement…</p>
	{:else if snapshots.length === 0}
		<p class="text-sm">Aucune sauvegarde automatique pour l’instant.</p>
	{:else}
		<ul class="flex flex-col divide-y divide-line rounded-lg border border-line text-sm">
			{#each snapshots as snapshot (snapshot.id)}
				<li class="flex flex-wrap items-center gap-x-4 gap-y-1 px-3 py-2">
					<div class="min-w-0 flex-1">
						<p class="font-medium">{snapshotDate(snapshot.createdAt)}</p>
						<p class="wrap-anywhere text-muted">
							Version {snapshot.appVersion}&nbsp;: {snapshot.sections
								.map((id) => sectionLabel(id, names))
								.join(', ')}
						</p>
					</div>
					<button
						type="button"
						class="secondary-button"
						aria-disabled={operation.busy}
						aria-label="Restaurer la sauvegarde du {snapshotDate(snapshot.createdAt)}"
						onclick={(event) => ask(snapshot, event.currentTarget)}
					>
						Restaurer…
					</button>
				</li>
			{/each}
		</ul>
	{/if}

	<p role="status" class="text-sm text-muted">
		{reading ? 'Lecture de la sauvegarde…' : operation.status}
	</p>
	{#if operation.problem}
		<ProblemAlert problem={operation.problem} lead="Sauvegarde automatique non restaurée." />
	{/if}
	{#if report}
		<BackupReport {report} {names} />
	{/if}
</section>

<ConfirmDialog
	open={pending !== undefined}
	title={`Restaurer cette sauvegarde${NBSP}?`}
	confirmLabel="Restaurer"
	onconfirm={restore}
	oncancel={() => (pending = undefined)}
	onclosed={() => opener?.focus()}
>
	{#if pending}
		<p class="mb-3">
			Sauvegarde du {snapshotDate(pending.snapshot.createdAt)}. L’état actuel est gardé avant d’être
			remplacé.
		</p>
		<BackupPreviewList preview={pending.preview} {names} />
	{/if}
</ConfirmDialog>
