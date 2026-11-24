<!--
@component
The identities by folder managed by HuBergamof, in the order they are written: each can be switched,
changed or removed (after a confirmation: the rule leaves the computer with it). The focus stays
where the person works: back to « Modifier » after a change, on `fallbackFocus` after a removal
(the rule that had the focus is gone).
-->
<script lang="ts">
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import type { GitFolderIdentity } from '#lib/generated/index.js';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import GitFolderForm from './GitFolderForm.svelte';
	import Switch from './Switch.svelte';

	interface Props {
		rules: readonly GitFolderIdentity[];
		onreplace: (folder: string, rule: GitFolderIdentity) => Promise<EditOutcome>;
		ontoggle: (folder: string, enabled: boolean) => void;
		onremove: (folder: string) => void;
		/** Where the focus goes once a rule is removed. */
		fallbackFocus?: () => void;
	}

	let { rules, onreplace, ontoggle, onremove, fallbackFocus }: Props = $props();

	let list: HTMLUListElement | undefined = $state();
	/** The folder whose form is open. */
	let editingFolder: string | undefined = $state();
	/** The folder whose « Modifier » button takes the focus as soon as it is displayed. */
	let editFocusFolder: string | undefined = $state();
	/** The rule waiting for the person to confirm its removal. */
	let removing: GitFolderIdentity | undefined = $state();

	// A renamed folder is a new row: the button exists once the list has the new folder, hence `rules`.
	$effect(() => {
		void rules;
		if (editFocusFolder === undefined) return;
		const button = rowOf(editFocusFolder)?.querySelector<HTMLButtonElement>('[data-edit]');
		if (!button) return;
		button.focus();
		editFocusFolder = undefined;
	});

	function closeEditor() {
		editFocusFolder = editingFolder;
		editingFolder = undefined;
	}

	async function replace(folder: string, changed: GitFolderIdentity): Promise<EditOutcome> {
		const outcome = await onreplace(folder, changed);
		if (outcome.saved) {
			editFocusFolder = changed.folder;
			editingFolder = undefined;
		}
		return outcome;
	}

	/** The neighbour whose « Supprimer » button takes the focus once the confirmation is closed (none: the last one). */
	let afterRemoval: { neighbour: string | undefined } | undefined;

	function rowOf(folder: string): HTMLLIElement | undefined {
		return [...(list?.querySelectorAll<HTMLLIElement>('li[data-folder]') ?? [])].find(
			(item) => item.dataset.folder === folder
		);
	}

	function confirmRemoval() {
		if (removing) {
			const index = rules.findIndex(({ folder }) => folder === removing?.folder);
			afterRemoval = { neighbour: (rules[index + 1] ?? rules[index - 1])?.folder };
			onremove(removing.folder);
		}
		removing = undefined;
	}

	/** After the dialog gave the focus back to its opener, which is about to leave. */
	function focusAfterRemoval() {
		if (!afterRemoval) return;
		const { neighbour } = afterRemoval;
		afterRemoval = undefined;
		const button =
			neighbour === undefined
				? undefined
				: rowOf(neighbour)?.querySelector<HTMLButtonElement>('[data-remove]');
		if (button) button.focus();
		else fallbackFocus?.();
	}

	/** What the rule sets, in words. */
	function summary(rule: GitFolderIdentity): string {
		return [
			rule.name && `nom ${rule.name}`,
			rule.email && rule.email,
			rule.signingKey && `clé ${rule.signingKey}`
		]
			.filter(Boolean)
			.join(' · ');
	}
</script>

{#if rules.length === 0}
	<p class="text-sm text-muted">Aucune identité par dossier pour l’instant.</p>
{:else}
	<ul bind:this={list} class="flex flex-col gap-2" aria-label="Identités par dossier">
		{#each rules as rule (rule.folder)}
			<li data-folder={rule.folder}>
				{#if editingFolder === rule.folder}
					<div class="rounded-xl border border-ring bg-sunken p-3">
						<GitFolderForm
							initial={rule}
							takenFolders={rules
								.filter(({ folder }) => folder !== rule.folder)
								.map(({ folder }) => folder)}
							submitLabel="Enregistrer"
							onsubmit={(changed) => replace(rule.folder, changed)}
							oncancel={closeEditor}
						/>
					</div>
				{:else}
					<div
						class="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border border-line p-3"
					>
						<div class={['flex min-w-0 flex-1 basis-60 flex-col', !rule.enabled && 'text-muted']}>
							<code class="font-mono font-semibold break-all">{rule.folder}</code>
							<span class="text-sm break-all">{summary(rule)}</span>
						</div>
						{#if !rule.enabled}
							<span class="rounded-full border border-line px-2 py-0.5 text-sm text-muted">
								Désactivée
							</span>
						{/if}
						<Switch
							checked={rule.enabled}
							label="Identité de {rule.folder}"
							onchange={(enabled) => ontoggle(rule.folder, enabled)}
						/>
						<div class="flex gap-2">
							<button
								type="button"
								data-edit
								class="secondary-button"
								aria-label="Modifier l’identité de {rule.folder}"
								onclick={() => (editingFolder = rule.folder)}
							>
								Modifier
							</button>
							<button
								type="button"
								data-remove
								class="secondary-button"
								aria-label="Supprimer l’identité de {rule.folder}"
								onclick={() => (removing = rule)}
							>
								Supprimer
							</button>
						</div>
					</div>
				{/if}
			</li>
		{/each}
	</ul>
{/if}

<ConfirmDialog
	open={removing !== undefined}
	title="Supprimer l’identité du dossier ?"
	confirmLabel="Supprimer"
	onconfirm={confirmRemoval}
	oncancel={() => (removing = undefined)}
	onclosed={focusAfterRemoval}
>
	L’identité du dossier <code class="font-mono font-semibold">{removing?.folder}</code> sera retirée de
	la configuration et de votre ordinateur.
</ConfirmDialog>
