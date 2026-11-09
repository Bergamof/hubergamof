<!--
@component
List of files, each with a name and an icon of the user's choice (compose files of the Docker
module). A file is added from the desktop's own file dialog, which gives its real path; where
there is none the path is typed. Files are edited and removed in place; like the other
settings, the changes are saved with « Appliquer ».
-->
<script lang="ts">
	import { tick, untrack } from 'svelte';
	import {
		IMAGE_TYPES,
		MAX_IMAGE_BYTES,
		keyImageUrl,
		mebibytes,
		type ManagerApi
	} from '#lib/api/manager-api.js';
	import { Operation } from '#lib/deck/operation.svelte.js';
	import type { Problem } from '#lib/deck/problems.js';
	import {
		MAX_ENTRY_NAME_CHARS,
		MAX_FILE_ENTRIES,
		fileEntryError,
		type FileEntryDraft
	} from '#lib/deck/settings-form.js';
	import ModuleTile from './ModuleTile.svelte';
	import ProblemAlert from './ProblemAlert.svelte';

	interface Props {
		/** Unique prefix of the element ids. */
		id: string;
		/** Id of the element naming the list. */
		labelledBy: string;
		/** Ids of the elements describing the list (help, error). */
		describedBy: string | undefined;
		entries: FileEntryDraft[];
		/** Device the icons are stored for. */
		serial: string;
		api: ManagerApi;
		/** Icon of a file that has none: the one of the module. */
		defaultIcon: string | null | undefined;
		/** The list is refused by the form: it takes the focus. */
		invalid: boolean;
		/** Called with the new list after every change. */
		onchange: (entries: FileEntryDraft[]) => void;
		/** Called when a file starts or stops being edited: the form is not applied meanwhile. */
		onediting: (editing: boolean) => void;
	}

	let {
		id,
		labelledBy,
		describedBy,
		entries,
		serial,
		api,
		defaultIcon,
		invalid,
		onchange,
		onediting
	}: Props = $props();

	/** The file being added (`index` is `null`) or edited, with the texts typed so far. */
	interface Editing {
		index: number | null;
		file: FileEntryDraft;
	}

	let root: HTMLDivElement | undefined = $state();
	let imageInput: HTMLInputElement | undefined = $state();
	let editing: Editing | undefined = $state();
	let editError: string | undefined = $state();
	/** The file dialog is open on the user's screen. */
	let choosing = $state(false);
	let chooserProblem: Problem | undefined = $state();
	let announcement = $state('');
	const iconUpload = new Operation();

	const full = $derived(entries.length >= MAX_FILE_ENTRIES);

	$effect(() => onediting(editing !== undefined));
	// The list changed from elsewhere (settings reloaded): what was being edited is gone.
	$effect(() => {
		void entries;
		untrack(() => (editing = undefined));
	});

	/** Name of the folder of `path`: what compose calls the project unless a name is given. */
	function folderName(path: string): string {
		const segments = path.split('/').filter((segment) => segment !== '' && segment !== '~');
		return segments.length >= 2 ? (segments[segments.length - 2] ?? path) : path;
	}

	function shownName(file: FileEntryDraft): string {
		return file.name.trim() || folderName(file.path.trim());
	}

	function iconOf(file: FileEntryDraft): string | null | undefined {
		return file.icon ? keyImageUrl(serial, file.icon) : defaultIcon;
	}

	async function focusOn(selector: string) {
		await tick();
		root?.querySelector<HTMLElement>(selector)?.focus();
	}

	function openEditor(index: number | null, file: FileEntryDraft) {
		if (editing) return;
		editing = { index, file: { ...file } };
		editError = undefined;
		iconUpload.clear();
		void focusOn('[data-field="path"]');
	}

	/** Opens the file dialog; `null` when it was closed, or could not open (then `chooserProblem`). */
	async function chooseFile(): Promise<string | null> {
		choosing = true;
		chooserProblem = undefined;
		const result = await api.chooseComposeFile();
		choosing = false;
		if (result.ok) return result.value;
		chooserProblem = result.problem;
		return null;
	}

	async function add() {
		if (choosing || full || editing) return;
		const path = await chooseFile();
		if (path !== null) {
			if (entries.some((entry) => entry.path.trim() === path)) {
				chooserProblem = { text: `« ${path} » est déjà dans la liste.`, detail: null };
				return;
			}
			openEditor(null, { path, name: '', icon: null });
		} else if (chooserProblem) {
			// No dialog on this desktop: the path is typed.
			openEditor(null, { path: '', name: '', icon: null });
		}
	}

	async function browse() {
		if (choosing || !editing) return;
		const path = await chooseFile();
		if (path !== null && editing) {
			editing.file.path = path;
			editError = undefined;
		}
	}

	function commit() {
		if (!editing || iconUpload.busy) return;
		const file = {
			path: editing.file.path.trim(),
			name: editing.file.name.trim(),
			icon: editing.file.icon
		};
		const { index } = editing;
		chooserProblem = undefined;
		const error =
			fileEntryError(file) ??
			(entries.some((other, position) => position !== index && other.path.trim() === file.path)
				? `« ${file.path} » est déjà dans la liste.`
				: undefined);
		if (error) {
			editError = error;
			void focusOn('[data-field="path"]');
			return;
		}
		const position = index ?? entries.length;
		onchange(index === null ? [...entries, file] : entries.map((e, i) => (i === index ? file : e)));
		editing = undefined;
		announce(
			index === null ? `« ${shownName(file)} » ajouté.` : `« ${shownName(file)} » modifié.`,
			index === null ? entries.length + 1 : entries.length
		);
		void focusOn(`[data-edit="${position}"]`);
	}

	function cancel() {
		if (!editing || iconUpload.busy) return;
		const { index } = editing;
		editing = undefined;
		chooserProblem = undefined;
		void focusOn(index === null ? '[data-add]' : `[data-edit="${index}"]`);
	}

	/** Says `message`; the count makes two identical messages differ, so both are read. */
	function announce(message: string, count: number) {
		announcement = `${message} ${count} ${count > 1 ? 'fichiers' : 'fichier'}.`;
	}

	function remove(index: number) {
		const file = entries[index];
		if (!file || editing) return;
		onchange(entries.filter((_, position) => position !== index));
		chooserProblem = undefined;
		announce(`« ${shownName(file)} » retiré de la liste.`, entries.length - 1);
		// The next file, else the previous one, else the button adding one.
		const neighbour = index < entries.length - 1 ? index : index - 1;
		void focusOn(neighbour >= 0 ? `[data-edit="${neighbour}"]` : '[data-add]');
	}

	/** Enter in a field confirms the file instead of submitting the whole settings form. */
	function confirmOnEnter(event: KeyboardEvent) {
		if (event.key !== 'Enter' || event.isComposing) return;
		event.preventDefault();
		commit();
	}

	function cancelOnEscape(event: KeyboardEvent) {
		if (event.key !== 'Escape' || event.isComposing) return;
		event.preventDefault();
		event.stopPropagation();
		cancel();
	}

	async function upload(file: File) {
		await iconUpload.run(async () => {
			const uploaded = await api.uploadKeyImage(serial, file);
			if (!uploaded.ok) return uploaded.problem;
			// Valider and Annuler wait for the upload: it is still the same edit.
			if (editing) editing.file.icon = uploaded.value;
			return undefined;
		}, 'Icône choisie.');
	}

	function picked(event: Event & { currentTarget: HTMLInputElement }) {
		const file = event.currentTarget.files?.[0];
		event.currentTarget.value = '';
		if (file) void upload(file);
	}

	function resetIcon() {
		if (!editing) return;
		editing.file.icon = null;
		iconUpload.clear();
		iconUpload.status = 'Icône par défaut rétablie.';
	}
</script>

<div
	bind:this={root}
	role="group"
	aria-labelledby={labelledBy}
	aria-describedby={describedBy}
	data-invalid={invalid ? 'true' : undefined}
	tabindex="-1"
	class="flex flex-col gap-3"
>
	{#if entries.length === 0 && !editing}
		<p class="text-sm text-muted">Aucun fichier ajouté.</p>
	{/if}

	{#if entries.length > 0 || editing}
		<ul class="flex flex-col gap-2">
			{#each entries as entry, index (index)}
				<li class="flex flex-col gap-3 card p-3">
					{#if editing && editing.index === index}
						{@render editor(editing)}
					{:else}
						<div class="flex flex-wrap items-center gap-3">
							<span class="w-14 shrink-0">
								<ModuleTile icon={iconOf(entry)} name={shownName(entry)} />
							</span>
							<div class="min-w-0 flex-1">
								<p class="font-medium break-words">
									{shownName(entry)}
									{#if !entry.name.trim()}
										<span class="text-sm font-normal text-muted">(nom par défaut)</span>
									{/if}
								</p>
								<p class="font-mono text-sm break-all text-muted">{entry.path}</p>
							</div>
							<div class="flex flex-wrap gap-2">
								<button
									type="button"
									class="secondary-button"
									data-edit={index}
									aria-label="Modifier {shownName(entry)}"
									aria-disabled={editing !== undefined}
									onclick={() => openEditor(index, entry)}
								>
									Modifier
								</button>
								<button
									type="button"
									class="danger-button"
									aria-label="Supprimer {shownName(entry)}"
									aria-disabled={editing !== undefined}
									onclick={() => remove(index)}
								>
									Supprimer
								</button>
							</div>
						</div>
					{/if}
				</li>
			{/each}
			{#if editing && editing.index === null}
				<li class="flex flex-col gap-3 card p-3">
					{@render editor(editing)}
				</li>
			{/if}
		</ul>
	{/if}

	<div class="flex flex-wrap items-center gap-2">
		<button
			type="button"
			class="secondary-button"
			data-add
			aria-disabled={choosing || full || editing !== undefined}
			aria-busy={choosing}
			onclick={() => void add()}
		>
			Ajouter un fichier…
		</button>
		{#if choosing}
			<span class="text-sm text-muted">Choisissez le fichier dans la fenêtre du bureau.</span>
		{/if}
	</div>
	{#if chooserProblem}
		<ProblemAlert problem={chooserProblem} />
	{/if}
	<p role="status" class="sr-only">
		{choosing
			? 'Sélecteur de fichier ouvert : choisissez le fichier dans la fenêtre du bureau.'
			: announcement}
	</p>
</div>

{#snippet editor(current: Editing)}
	<!-- Escape is heard from the fields inside: the group itself takes no input. -->
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<div
		role="group"
		aria-label={current.index === null ? 'Nouveau fichier' : `Modifier ${shownName(current.file)}`}
		class="flex flex-col gap-3"
		onkeydown={cancelOnEscape}
	>
		<div class="flex flex-col gap-1">
			<label class="font-medium" for="{id}-path">Fichier</label>
			<span class="flex flex-wrap items-center gap-2">
				<input
					id="{id}-path"
					data-field="path"
					type="text"
					spellcheck="false"
					class="field min-w-0 flex-1 font-mono"
					placeholder="/chemin/vers/compose.yaml"
					bind:value={current.file.path}
					oninput={() => (editError = undefined)}
					onkeydown={confirmOnEnter}
					aria-describedby="{id}-path-hint{editError ? ` ${id}-edit-error` : ''}"
					aria-invalid={editError ? true : undefined}
				/>
				<button
					type="button"
					class="secondary-button"
					aria-disabled={choosing}
					onclick={() => void browse()}
				>
					Parcourir…
				</button>
			</span>
			<p id="{id}-path-hint" class="text-sm text-muted">
				Chemin absolu (ou commençant par ~/), choisi avec « Parcourir » ou saisi.
			</p>
		</div>

		<div class="flex flex-col gap-1">
			<label class="font-medium" for="{id}-name">Nom</label>
			<input
				id="{id}-name"
				type="text"
				class="field"
				placeholder={folderName(current.file.path.trim()) || 'Nom du projet'}
				bind:value={current.file.name}
				onkeydown={confirmOnEnter}
				aria-describedby="{id}-name-hint"
			/>
			<p id="{id}-name-hint" class="text-sm text-muted">
				Vide : nom du projet compose. {MAX_ENTRY_NAME_CHARS} caractères au plus.
			</p>
		</div>

		<div class="flex flex-col gap-2">
			<span class="font-medium" id="{id}-icon-title">Icône</span>
			<div class="flex flex-wrap items-center gap-3">
				<span class="w-14 shrink-0">
					<ModuleTile icon={iconOf(current.file)} name={shownName(current.file)} />
				</span>
				<div class="flex flex-wrap gap-2">
					<button
						type="button"
						class="secondary-button"
						aria-disabled={iconUpload.busy}
						aria-describedby="{id}-icon-hint"
						onclick={() => iconUpload.busy || imageInput?.click()}
					>
						{iconUpload.busy ? 'Envoi…' : 'Choisir une image…'}
					</button>
					{#if current.file.icon}
						<button
							type="button"
							class="secondary-button"
							aria-disabled={iconUpload.busy}
							onclick={() => iconUpload.busy || resetIcon()}
						>
							Rétablir l’icône par défaut
						</button>
					{/if}
				</div>
			</div>
			<p id="{id}-icon-hint" class="text-sm text-muted">
				PNG, JPEG ou SVG, {mebibytes(MAX_IMAGE_BYTES)} au plus. {current.file.icon
					? 'Image personnalisée.'
					: 'Icône par défaut.'}
			</p>
			<input
				bind:this={imageInput}
				type="file"
				accept={IMAGE_TYPES.join(',')}
				class="hidden"
				tabindex="-1"
				aria-hidden="true"
				onchange={picked}
			/>
			<p role="status" class="text-sm text-muted">{iconUpload.status}</p>
			{#if iconUpload.problem}
				<ProblemAlert problem={iconUpload.problem} />
			{/if}
		</div>

		{#if editError}
			<p id="{id}-edit-error" role="alert" class="text-sm font-medium text-danger">{editError}</p>
		{/if}
		<div class="flex flex-wrap gap-2">
			<button type="button" class="primary-button" aria-disabled={iconUpload.busy} onclick={commit}>
				{current.index === null ? 'Ajouter à la liste' : 'Valider'}
			</button>
			<button
				type="button"
				class="secondary-button"
				aria-disabled={iconUpload.busy}
				onclick={cancel}
			>
				Annuler
			</button>
		</div>
	</div>
{/snippet}
