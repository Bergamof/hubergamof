<!--
@component
The SSH hosts of the machine: the short names the person connects with. What HuBergamof manages is
written in a file of its own, included from the very top of `~/.ssh/config`, which it only completes.
The computer is read when the page opens (and after a reconnection), never watched. Every change is
sent at once; the line under the title says whether the manager has saved it.
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import type { DeckState } from '#lib/deck/deck-state.svelte.js';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import { SaveTracker } from '#lib/deck/save-tracker.svelte.js';
	import {
		addHost,
		isHostListFull,
		removeHost,
		replaceHost,
		setHostEnabled,
		type SshEdit
	} from '#lib/deck/ssh-edits.js';
	import { draftOf, hostOf } from '#lib/deck/ssh-validation.js';
	import type { SshSystemHost } from '#lib/generated/index.js';
	import SaveStatusLine from './SaveStatusLine.svelte';
	import SshHostForm from './SshHostForm.svelte';
	import SshHostList from './SshHostList.svelte';
	import SshSystemHosts from './SshSystemHosts.svelte';

	interface Props {
		deck: DeckState;
	}

	let { deck }: Props = $props();

	const uid = $props.id();
	let addForm: ReturnType<typeof SshHostForm> | undefined = $state();
	let addHeading: HTMLHeadingElement | undefined = $state();
	let systemHeading: HTMLHeadingElement | undefined = $state();
	const saves = new SaveTracker();
	const config = $derived(deck.ssh.config);
	/** Hosts of the system that are not in the configuration yet: taken, one leaves the list at once. */
	const unmanaged = $derived(
		deck.ssh.systemHosts.filter(
			(system) => !config?.hosts.some((host) => host.alias === system.alias)
		)
	);

	// Read once on opening; `watch` sends, which reads the connection status: not a reason to read again.
	$effect(() => untrack(() => deck.ssh.watch()));

	function edit(change: SshEdit): Promise<EditOutcome> {
		const outcome = deck.ssh.edit(change);
		void saves.track(outcome);
		return outcome;
	}

	function adopt(system: SshSystemHost) {
		void edit(addHost(hostOf({ ...draftOf({ ...system, enabled: true }) })));
	}
</script>

<div class="flex flex-col gap-8">
	<SaveStatusLine tracker={saves} />

	<section aria-labelledby="{uid}-hosts" class="flex flex-col gap-4 card">
		<h2 id="{uid}-hosts" class="font-display text-xl font-semibold">Serveurs SSH</h2>
		<p class="text-sm text-muted">
			Écrits dans un fichier de HuBergamof, inclus tout en haut de
			<code class="font-mono">~/.ssh/config</code>, qui reste sinon tel que vous l’avez écrit : ce
			qui est écrit ici l’emporte. Seules les options qui ne lancent aucun programme sont gérées
			(pas de <code class="font-mono">ProxyCommand</code>). Un serveur désactivé reste ici mais
			n’est plus sur l’ordinateur.
		</p>
		{#if config}
			<SshHostList
				hosts={config.hosts}
				onreplace={(alias, host) => edit(replaceHost(alias, host))}
				ontoggle={(alias, enabled) => void edit(setHostEnabled(alias, enabled))}
				onremove={(alias) => void edit(removeHost(alias))}
				fallbackFocus={() => (addForm ? addForm.focusAlias() : addHeading?.focus())}
			/>
			<h3
				bind:this={addHeading}
				tabindex="-1"
				id="{uid}-add"
				class="font-display text-lg font-semibold"
			>
				Ajouter un serveur
			</h3>
			{#if isHostListFull(config)}
				<p class="text-sm text-muted">
					La configuration contient déjà le nombre maximal de serveurs : supprimez-en un pour en
					ajouter.
				</p>
			{:else}
				<SshHostForm
					bind:this={addForm}
					labelledby="{uid}-add"
					takenAliases={config.hosts.map(({ alias }) => alias)}
					submitLabel="Ajouter"
					onsubmit={(host) => edit(addHost(host))}
				/>
			{/if}
		{:else}
			<p class="text-muted" role="status" aria-busy="true">
				Chargement de la configuration de SSH…
			</p>
		{/if}
	</section>

	{#if config}
		<section aria-labelledby="{uid}-found" class="flex flex-col gap-4 card">
			<h2
				bind:this={systemHeading}
				tabindex="-1"
				id="{uid}-found"
				class="font-display text-xl font-semibold"
			>
				Serveurs définis ailleurs, non gérés
			</h2>
			<p class="text-sm text-muted">
				Seul <code class="font-mono">~/.ssh/config</code> est lu, au moment où cette page s’ouvre ;
				les fichiers qu’il inclut ne sont pas suivis. « Reprendre » ajoute le serveur à la
				configuration, activé, et commente sa section d’origine (préfixe
				<code class="font-mono">#hubergamof-off#</code>) : désactivé ici, il disparaît vraiment ;
				supprimé de la configuration, il retrouve sa section.
			</p>
			<SshSystemHosts
				hosts={unmanaged}
				takenAliases={config.hosts.map(({ alias }) => alias)}
				full={isHostListFull(config)}
				onadopt={adopt}
				fallbackFocus={() => systemHeading?.focus()}
			/>
		</section>
	{/if}
</div>
