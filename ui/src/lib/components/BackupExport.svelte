<!--
@component
Export of the configuration of the application to a zip: what to put in it is ticked by kind of
document. The file holds the Git identity, the SSH hosts and the commands of the aliases: the
page says so before it is saved. It is fetched, not linked, so that a refusal (computer locked)
is shown instead of being saved as a file.
-->
<script lang="ts">
	import {
		BACKUP_GROUPS,
		backupFileName,
		type BackupGroup,
		type ManagerApi
	} from '#lib/api/manager-api.js';
	import { GROUP_LABELS } from '#lib/deck/backup-labels.js';
	import { Operation } from '#lib/deck/operation.svelte.js';
	import ProblemAlert from './ProblemAlert.svelte';

	interface Props {
		api: ManagerApi;
	}

	let { api }: Props = $props();

	const uid = $props.id();
	const operation = new Operation();
	/** Ticked kinds: everything, as an export is meant to be the whole of the configuration. */
	let ticked: BackupGroup[] = $state([...BACKUP_GROUPS]);
	const empty = $derived(ticked.length === 0);

	/** Time the browser is given to start the download before the object is released. */
	const RELEASE_AFTER_MS = 1000;

	function save(blob: Blob) {
		const url = URL.createObjectURL(blob);
		const link = document.createElement('a');
		link.href = url;
		link.download = backupFileName(new Date());
		link.click();
		setTimeout(() => URL.revokeObjectURL(url), RELEASE_AFTER_MS);
	}

	function toggle(group: BackupGroup, checked: boolean) {
		ticked = BACKUP_GROUPS.filter((one) => (one === group ? checked : ticked.includes(one)));
	}

	async function exportNow() {
		if (operation.busy || empty) return;
		const groups = ticked.length === BACKUP_GROUPS.length ? undefined : ticked;
		await operation.run(async () => {
			const result = await api.exportBackup(groups);
			if (!result.ok) return result.problem;
			save(result.value);
			return undefined;
		}, 'Sauvegarde exportée : elle est dans vos téléchargements.');
	}
</script>

<section aria-labelledby="{uid}-title" class="flex flex-col gap-3 card">
	<h2 id="{uid}-title" class="text-lg font-semibold">Exporter</h2>
	<p class="text-sm text-muted">
		Un fichier zip avec la configuration de HuBergamof, pour la restaurer ici ou sur un autre
		ordinateur.
	</p>
	<p class="rounded-lg border border-edge bg-sunken p-3 text-sm">
		<strong class="font-semibold">Gardez ce fichier privé.</strong> Il contient votre identité Git, vos
		hôtes et utilisateurs SSH et les commandes de vos alias : ne le laissez pas dans un dossier partagé.
		Il ne contient aucune clé privée ni les réglages de mises à jour.
	</p>
	<fieldset class="flex flex-col gap-1">
		<legend class="mb-1 text-sm font-medium">Ce que la sauvegarde contient</legend>
		<div class="grid grid-cols-2 gap-x-6 gap-y-1 @xl:grid-cols-4">
			{#each BACKUP_GROUPS as group (group)}
				<label class="flex min-h-8 items-center gap-2 text-sm" for="{uid}-{group}">
					<input
						id="{uid}-{group}"
						type="checkbox"
						class="size-5 accent-ring focus-ring"
						checked={ticked.includes(group)}
						onchange={(event) => toggle(group, event.currentTarget.checked)}
					/>
					{GROUP_LABELS[group]}
				</label>
			{/each}
		</div>
	</fieldset>
	<div class="flex flex-wrap items-center gap-3">
		<!-- `aria-disabled`, not `disabled`: the button keeps the focus while it works. -->
		<button
			type="button"
			class="secondary-button"
			aria-disabled={operation.busy || empty}
			aria-describedby={empty ? `${uid}-empty` : undefined}
			onclick={exportNow}
		>
			{operation.busy ? 'Export…' : 'Exporter la configuration'}
		</button>
		{#if empty}
			<p id="{uid}-empty" class="text-sm text-muted">Cochez au moins une case.</p>
		{/if}
	</div>
	<p role="status" class="text-sm text-muted">{operation.status}</p>
	{#if operation.problem}
		<ProblemAlert problem={operation.problem} lead="Sauvegarde non exportée." />
	{/if}
</section>
