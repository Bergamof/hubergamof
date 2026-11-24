<!--
@component
The shell aliases of the machine: those HuBergamof keeps in its own block of the startup files, and those
the manager found there that it does not manage. The system is read when the page opens (and
after a reconnection), never watched. Every change is sent at once; the line under the title says
whether the manager has saved it.
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import { hasErrors, validateAlias } from '#lib/deck/alias-validation.js';
	import type { DeckState } from '#lib/deck/deck-state.svelte.js';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import {
		addAlias,
		isFull,
		removeAlias,
		replaceAlias,
		setAliasEnabled,
		type MachineEdit
	} from '#lib/deck/machine-edits.js';
	import { SaveTracker } from '#lib/deck/save-tracker.svelte.js';
	import type { ShellAlias, SystemAlias } from '#lib/generated/index.js';
	import AliasForm from './AliasForm.svelte';
	import AliasList from './AliasList.svelte';
	import SaveStatusLine from './SaveStatusLine.svelte';
	import SystemAliasList from './SystemAliasList.svelte';

	interface Props {
		deck: DeckState;
	}

	let { deck }: Props = $props();

	const uid = $props.id();
	let addForm: ReturnType<typeof AliasForm> | undefined = $state();
	let addHeading: HTMLHeadingElement | undefined = $state();
	let systemHeading: HTMLHeadingElement | undefined = $state();
	const saves = new SaveTracker();
	const config = $derived(deck.machine.config);
	/** Aliases of the system that are not in the configuration yet: taken in, one leaves the list at once. */
	const unmanaged = $derived(
		deck.machine.systemAliases.filter(
			(system) => !config?.aliases.some((alias) => alias.name === system.name)
		)
	);

	// Read once on opening; `watch` sends, which reads the connection status: not a reason to read again.
	$effect(() => untrack(() => deck.machine.watch()));

	function edit(change: MachineEdit): Promise<EditOutcome> {
		const outcome = deck.machine.edit(change);
		void saves.track(outcome);
		return outcome;
	}

	function adopt({ name, command }: SystemAlias) {
		const alias: ShellAlias = { name, command, enabled: true };
		if (!hasErrors(validateAlias(alias, []))) void edit(addAlias(alias));
	}
</script>

<div class="flex flex-col gap-8">
	<SaveStatusLine tracker={saves} />

	<section aria-labelledby="{uid}-managed" class="flex flex-col gap-4 card">
		<h2 id="{uid}-managed" class="font-display text-xl font-semibold">Alias du shell</h2>
		<p class="text-sm text-muted">
			Écrits dans un bloc réservé de <code class="font-mono">~/.bashrc</code> et
			<code class="font-mono">~/.zshrc</code>, ils sont disponibles dans les nouveaux terminaux. Un
			alias désactivé reste ici mais n’est plus sur l’ordinateur.
		</p>
		{#if config}
			<AliasList
				aliases={config.aliases}
				onreplace={(name, alias) => edit(replaceAlias(name, alias))}
				ontoggle={(name, enabled) => void edit(setAliasEnabled(name, enabled))}
				onremove={(name) => void edit(removeAlias(name))}
				fallbackFocus={() => (addForm ? addForm.focusName() : addHeading?.focus())}
			/>
			<h3
				bind:this={addHeading}
				tabindex="-1"
				id="{uid}-add"
				class="font-display text-lg font-semibold"
			>
				Ajouter un alias
			</h3>
			{#if isFull(config)}
				<p class="text-sm text-muted">
					La configuration contient déjà le nombre maximal d’alias : supprimez-en un pour en
					ajouter.
				</p>
			{:else}
				<AliasForm
					bind:this={addForm}
					labelledby="{uid}-add"
					takenNames={config.aliases.map(({ name }) => name)}
					submitLabel="Ajouter"
					onsubmit={(alias) => edit(addAlias(alias))}
				/>
			{/if}
		{:else}
			<p class="text-muted">Chargement des alias…</p>
		{/if}
	</section>

	<section aria-labelledby="{uid}-system" class="flex flex-col gap-4 card">
		<h2
			bind:this={systemHeading}
			tabindex="-1"
			id="{uid}-system"
			class="font-display text-xl font-semibold"
		>
			Alias présents sur le système, non gérés
		</h2>
		<p class="text-sm text-muted">
			Seuls <code class="font-mono">~/.bashrc</code>, <code class="font-mono">~/.zshrc</code>,
			<code class="font-mono">~/.bash_aliases</code> et
			<code class="font-mono">~/.zsh_aliases</code>
			sont lus, au moment où cette page s’ouvre. « Intégrer » ajoute l’alias à la configuration, activé,
			et commente sa ligne d’origine (préfixe <code class="font-mono">: #hubergamof-off#</code>) ;
			supprimer l’alias de la configuration la rétablit. Un alias de ce formulaire fait de même pour
			un alias de même nom déjà présent.
		</p>
		{#if config}
			<SystemAliasList
				aliases={unmanaged}
				full={isFull(config)}
				onadopt={adopt}
				fallbackFocus={() => systemHeading?.focus()}
			/>
		{/if}
	</section>
</div>
