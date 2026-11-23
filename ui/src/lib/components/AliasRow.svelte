<!--
@component
One alias managed by HuBergamof: what it runs, whether it is on the computer, and the buttons to
change it or remove it. Changing it opens the alias form in its place.
-->
<script lang="ts">
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import type { ShellAlias } from '#lib/generated/index.js';
	import AliasForm from './AliasForm.svelte';
	import Switch from './Switch.svelte';

	interface Props {
		alias: ShellAlias;
		/** Names of the other aliases. */
		takenNames: readonly string[];
		onreplace: (alias: ShellAlias) => Promise<EditOutcome>;
		ontoggle: (enabled: boolean) => void;
		onremove: () => void;
	}

	let { alias, takenNames, onreplace, ontoggle, onremove }: Props = $props();

	let editing = $state(false);
	let editButton: HTMLButtonElement | undefined = $state();

	function closeEditor() {
		editing = false;
		// The form held the focus: the button that opened it takes it back, once displayed.
		queueMicrotask(() => editButton?.focus());
	}
</script>

{#if editing}
	<div class="rounded-xl border border-ring bg-sunken p-3">
		<AliasForm
			initial={alias}
			{takenNames}
			submitLabel="Enregistrer"
			onsubmit={onreplace}
			ondone={closeEditor}
			oncancel={closeEditor}
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
		<Switch checked={alias.enabled} label="Activer l’alias {alias.name}" onchange={ontoggle} />
		<div class="flex gap-2">
			<button
				bind:this={editButton}
				type="button"
				class="secondary-button"
				aria-label="Modifier l’alias {alias.name}"
				onclick={() => (editing = true)}
			>
				Modifier
			</button>
			<button
				type="button"
				class="secondary-button"
				aria-label="Supprimer l’alias {alias.name}"
				onclick={onremove}
			>
				Supprimer
			</button>
		</div>
	</div>
{/if}
