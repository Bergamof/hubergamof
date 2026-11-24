<!--
@component
One alias managed by HuBergamof: what it runs, whether it is on the computer, and the buttons to
change it or remove it. Changing it opens the alias form in its place; the list says when (it also moves the focus).
-->
<script lang="ts">
	import { SHELL_ALIAS_RULES, type EntryRules } from '#lib/deck/alias-rules.js';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import type { ShellAlias } from '#lib/generated/index.js';
	import AliasForm from './AliasForm.svelte';
	import Switch from './Switch.svelte';

	interface Props {
		alias: ShellAlias;
		/** The alias form is open in place of the row. */
		editing: boolean;
		/** Names of the other aliases. */
		takenNames: readonly string[];
		onedit: () => void;
		/** The form was given up. */
		oncancel: () => void;
		onreplace: (alias: ShellAlias) => Promise<EditOutcome>;
		ontoggle: (enabled: boolean) => void;
		onremove: () => void;
		/** What the aliases must follow; the shell's by default. */
		rules?: EntryRules;
		/** Where the entry belongs, when several lists look alike (« du serveur prod »), for the names of its buttons. */
		suffix?: string;
	}

	let {
		alias,
		editing,
		takenNames,
		onedit,
		oncancel,
		onreplace,
		ontoggle,
		onremove,
		rules = SHELL_ALIAS_RULES,
		suffix = ''
	}: Props = $props();
</script>

{#if editing}
	<div class="rounded-xl border border-ring bg-sunken p-3">
		<AliasForm
			initial={alias}
			{takenNames}
			submitLabel="Enregistrer"
			onsubmit={onreplace}
			{oncancel}
			{rules}
		/>
	</div>
{:else}
	<div class="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border border-line p-3">
		<div class={['flex min-w-0 flex-1 basis-60 flex-col', !alias.enabled && 'text-muted']}>
			<code class="font-mono font-semibold break-all">{alias.name}</code>
			<code class="font-mono text-sm break-all">{alias.command}</code>
		</div>
		{#if !alias.enabled}
			<span class="rounded-full border border-line px-2 py-0.5 text-sm text-muted">Désactivé</span>
		{/if}
		<Switch
			checked={alias.enabled}
			label="{rules.title} {alias.name}{suffix}"
			onchange={ontoggle}
		/>
		<div class="flex gap-2">
			<button
				data-edit
				type="button"
				class="secondary-button"
				aria-label="Modifier l’{rules.noun} {alias.name}{suffix}"
				onclick={onedit}
			>
				Modifier
			</button>
			<button
				type="button"
				data-remove
				class="secondary-button"
				aria-label="Supprimer l’{rules.noun} {alias.name}{suffix}"
				onclick={onremove}
			>
				Supprimer
			</button>
		</div>
	</div>
{/if}
