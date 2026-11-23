<!--
@component
The aliases managed by HuBergamof, in the order they are written: each can be switched, changed
or removed (after a confirmation: the alias leaves the computer with it).
-->
<script lang="ts">
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import type { ShellAlias } from '#lib/generated/index.js';
	import AliasRow from './AliasRow.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';

	interface Props {
		aliases: readonly ShellAlias[];
		onreplace: (name: string, alias: ShellAlias) => Promise<EditOutcome>;
		ontoggle: (name: string, enabled: boolean) => void;
		onremove: (name: string) => void;
	}

	let { aliases, onreplace, ontoggle, onremove }: Props = $props();

	/** The alias waiting for the person to confirm its removal. */
	let removing: ShellAlias | undefined = $state();

	function confirmRemoval() {
		if (removing) onremove(removing.name);
		removing = undefined;
	}
</script>

{#if aliases.length === 0}
	<p class="text-sm text-muted">Aucun alias géré pour l’instant.</p>
{:else}
	<ul class="flex flex-col gap-2" aria-label="Alias gérés">
		{#each aliases as alias (alias.name)}
			<li>
				<AliasRow
					{alias}
					takenNames={aliases.filter(({ name }) => name !== alias.name).map(({ name }) => name)}
					onreplace={(changed) => onreplace(alias.name, changed)}
					ontoggle={(enabled) => ontoggle(alias.name, enabled)}
					onremove={() => (removing = alias)}
				/>
			</li>
		{/each}
	</ul>
{/if}

<ConfirmDialog
	open={removing !== undefined}
	title="Supprimer l’alias ?"
	confirmLabel="Supprimer"
	onconfirm={confirmRemoval}
	oncancel={() => (removing = undefined)}
>
	L’alias <code class="font-mono font-semibold">{removing?.name}</code> sera retiré de la configuration
	et de votre ordinateur.
</ConfirmDialog>
