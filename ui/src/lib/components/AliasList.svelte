<!--
@component
The aliases managed by HuBergamof, in the order they are written: each can be switched, changed
or removed (after a confirmation: the alias leaves the computer with it). The list moves the focus
where the person is working after each of these: back to « Modifier » of the alias, to « Supprimer »
of a neighbour (or to `fallbackFocus` when it was the last one).
-->
<script lang="ts">
	import { SHELL_ALIAS_RULES, type AliasRules } from '#lib/deck/alias-rules.js';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import type { ShellAlias } from '#lib/generated/index.js';
	import AliasRow from './AliasRow.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';

	interface Props {
		aliases: readonly ShellAlias[];
		onreplace: (name: string, alias: ShellAlias) => Promise<EditOutcome>;
		ontoggle: (name: string, enabled: boolean) => void;
		onremove: (name: string) => void;
		/** Where the focus goes once the last alias is removed. */
		fallbackFocus?: () => void;
		/** What the aliases must follow; the shell's by default. */
		rules?: AliasRules;
	}

	let {
		aliases,
		onreplace,
		ontoggle,
		onremove,
		fallbackFocus,
		rules = SHELL_ALIAS_RULES
	}: Props = $props();

	let list: HTMLUListElement | undefined = $state();
	/** The alias whose form is open. */
	let editingName: string | undefined = $state();
	/** The alias whose « Modifier » button takes the focus as soon as it is displayed. */
	let editFocusName: string | undefined = $state();
	/** The neighbour whose « Supprimer » button takes the focus once the confirmation is closed (none: the last one). */
	let afterRemoval: { neighbour: string | undefined } | undefined;

	function rowOf(name: string): HTMLLIElement | undefined {
		return [...(list?.querySelectorAll<HTMLLIElement>('li[data-alias]') ?? [])].find(
			(row) => row.dataset.alias === name
		);
	}

	// A renamed alias is a new row: the button exists once the list has the new name, hence `aliases`.
	$effect(() => {
		void aliases;
		if (editFocusName === undefined) return;
		const button = rowOf(editFocusName)?.querySelector<HTMLButtonElement>('[data-edit]');
		if (!button) return;
		button.focus();
		editFocusName = undefined;
	});

	function closeEditor() {
		editFocusName = editingName;
		editingName = undefined;
	}

	async function replace(name: string, changed: ShellAlias): Promise<EditOutcome> {
		const outcome = await onreplace(name, changed);
		if (outcome.saved) {
			editFocusName = changed.name;
			editingName = undefined;
		}
		return outcome;
	}

	/** The alias waiting for the person to confirm its removal. */
	let removing: ShellAlias | undefined = $state();

	function confirmRemoval() {
		if (removing) {
			const index = aliases.findIndex(({ name }) => name === removing?.name);
			afterRemoval = { neighbour: (aliases[index + 1] ?? aliases[index - 1])?.name };
			onremove(removing.name);
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
</script>

{#if aliases.length === 0}
	<p class="text-sm text-muted">Aucun {rules.noun} géré pour l’instant.</p>
{:else}
	<ul bind:this={list} class="flex flex-col gap-2" aria-label="{rules.title} gérés">
		{#each aliases as alias (alias.name)}
			<li data-alias={alias.name}>
				<AliasRow
					{alias}
					editing={editingName === alias.name}
					takenNames={aliases.filter(({ name }) => name !== alias.name).map(({ name }) => name)}
					onedit={() => (editingName = alias.name)}
					oncancel={closeEditor}
					onreplace={(changed) => replace(alias.name, changed)}
					ontoggle={(enabled) => ontoggle(alias.name, enabled)}
					onremove={() => (removing = alias)}
					{rules}
				/>
			</li>
		{/each}
	</ul>
{/if}

<ConfirmDialog
	open={removing !== undefined}
	title="Supprimer l’{rules.noun} ?"
	confirmLabel="Supprimer"
	onconfirm={confirmRemoval}
	oncancel={() => (removing = undefined)}
	onclosed={focusAfterRemoval}
>
	L’{rules.noun} <code class="font-mono font-semibold">{removing?.name}</code> sera retiré de la configuration
	et de votre ordinateur.
</ConfirmDialog>
