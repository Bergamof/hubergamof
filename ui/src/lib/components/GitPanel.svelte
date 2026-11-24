<!--
@component
The Git configuration of the machine: who the commits are made as, how they are signed, how Git
behaves, its aliases and the identities that depend on the folder of a repository. What HuBergamof
manages is written in files of its own, included from the Git configuration of the user, which it
only completes. The computer is read when the page opens (and after a reconnection), never watched.
Each form is sent as a whole; the line under the title says whether the manager has saved it.
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import { GIT_ALIAS_RULES } from '#lib/deck/alias-rules.js';
	import { hasErrors } from '#lib/deck/alias-validation.js';
	import type { DeckState } from '#lib/deck/deck-state.svelte.js';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import {
		addFolderIdentity,
		isFolderListFull,
		removeFolderIdentity,
		replaceFolderIdentity,
		setFolderIdentityEnabled,
		setIdentity,
		setSettings,
		setSigning,
		type GitEdit
	} from '#lib/deck/git-edits.js';
	import { GIT_ALIASES_MAX } from '#lib/deck/git-validation.js';
	import {
		addAlias,
		isFull,
		removeAlias,
		replaceAlias,
		setAliasEnabled
	} from '#lib/deck/machine-edits.js';
	import { SaveTracker } from '#lib/deck/save-tracker.svelte.js';
	import type { GitConfig, ShellAlias, SystemAlias } from '#lib/generated/index.js';
	import AliasForm from './AliasForm.svelte';
	import AliasList from './AliasList.svelte';
	import GitFolderForm from './GitFolderForm.svelte';
	import GitFolderList from './GitFolderList.svelte';
	import GitIdentityForm from './GitIdentityForm.svelte';
	import GitSettingsForm from './GitSettingsForm.svelte';
	import GitSigningForm from './GitSigningForm.svelte';
	import GitSystemEntries from './GitSystemEntries.svelte';
	import SaveStatusLine from './SaveStatusLine.svelte';
	import SystemAliasList from './SystemAliasList.svelte';

	interface Props {
		deck: DeckState;
	}

	let { deck }: Props = $props();

	/** What names an alias in the keys Git lists (`alias.co`). */
	const ALIAS_KEY_PREFIX = 'alias.';

	/** The files of the user whose lines the manager comments out (`GLOBAL_FILES` of `apply.rs`). */
	const CLAIMABLE_ORIGINS = ['~/.gitconfig', '~/.config/git/config'];

	/** An alias defined in another file could not be switched off: its line would stay. */
	function refuseOtherFile({ source }: SystemAlias): string | undefined {
		return CLAIMABLE_ORIGINS.includes(source)
			? undefined
			: `Défini dans ${source} : HuBergamof ne modifie que ~/.gitconfig et ~/.config/git/config, cet alias ne pourrait pas être vraiment désactivé.`;
	}

	const uid = $props.id();
	let addForm: ReturnType<typeof AliasForm> | undefined = $state();
	let addAliasHeading: HTMLHeadingElement | undefined = $state();
	let systemAliasHeading: HTMLHeadingElement | undefined = $state();
	let addFolderForm: ReturnType<typeof GitFolderForm> | undefined = $state();
	let foldersHeading: HTMLHeadingElement | undefined = $state();
	const saves = new SaveTracker();
	const config = $derived(deck.git.config);

	/** The aliases of the other Git files, as the list of the system aliases shows them. */
	const foundAliases = $derived<SystemAlias[]>(
		deck.git.systemEntries
			.filter(({ key }) => key.startsWith(ALIAS_KEY_PREFIX))
			.map(({ key, value, origin }) => ({
				name: key.slice(ALIAS_KEY_PREFIX.length),
				command: value,
				source: origin
			}))
	);
	/** Those not in the configuration yet: taken in, one leaves the list at once. */
	const unmanagedAliases = $derived(
		foundAliases.filter(
			({ name }) =>
				!config?.aliases.some((alias) => alias.name.toLowerCase() === name.toLowerCase())
		)
	);
	const otherEntries = $derived(
		deck.git.systemEntries.filter(({ key }) => !key.startsWith(ALIAS_KEY_PREFIX))
	);

	// Read once on opening; `watch` sends, which reads the connection status: not a reason to read again.
	$effect(() => untrack(() => deck.git.watch()));

	function edit(change: GitEdit): Promise<EditOutcome> {
		const outcome = deck.git.edit(change);
		void saves.track(outcome);
		return outcome;
	}

	function adopt({ name, command }: SystemAlias) {
		const alias: ShellAlias = { name, command, enabled: true };
		if (!hasErrors(GIT_ALIAS_RULES.validate(alias, []))) {
			void edit(addAlias<GitConfig>(alias, GIT_ALIASES_MAX));
		}
	}
</script>

<div class="flex flex-col gap-8">
	<SaveStatusLine tracker={saves} />

	{#if deck.git.gitAvailable === false}
		<p class="rounded-xl border border-danger bg-danger-soft p-3 text-sm text-danger" role="alert">
			Le programme <code class="font-mono">git</code> est introuvable : les valeurs déjà présentes sur
			l’ordinateur ne peuvent pas être lues. La configuration ci-dessous reste enregistrée et appliquée.
		</p>
	{/if}

	<section aria-labelledby="{uid}-identity" class="flex flex-col gap-4 card">
		<h2 id="{uid}-identity" class="font-display text-xl font-semibold">Identité</h2>
		<p class="text-sm text-muted">
			Écrite dans un fichier de HuBergamof, inclus à la fin de
			<code class="font-mono">~/.gitconfig</code>, qui reste tel que vous l’avez écrit. Ce qui est
			écrit ici l’emporte sur le reste ; un champ vide laisse Git comme il est.
		</p>
		{#if config}
			<GitIdentityForm
				identity={config.identity}
				labelledby="{uid}-identity"
				onsubmit={(identity) => edit(setIdentity(identity))}
			/>
		{:else}
			<p class="text-muted" role="status" aria-busy="true">
				Chargement de la configuration de Git…
			</p>
		{/if}
	</section>

	{#if config}
		<section aria-labelledby="{uid}-signing" class="flex flex-col gap-4 card">
			<h2 id="{uid}-signing" class="font-display text-xl font-semibold">Signature des commits</h2>
			<GitSigningForm
				signing={config.signing}
				labelledby="{uid}-signing"
				onsubmit={(signing) => edit(setSigning(signing))}
			/>
		</section>

		<section aria-labelledby="{uid}-settings" class="flex flex-col gap-4 card">
			<h2 id="{uid}-settings" class="font-display text-xl font-semibold">Réglages</h2>
			<GitSettingsForm
				settings={config.settings}
				labelledby="{uid}-settings"
				onsubmit={(settings) => edit(setSettings(settings))}
			/>
		</section>

		<section aria-labelledby="{uid}-aliases" class="flex flex-col gap-4 card">
			<h2 id="{uid}-aliases" class="font-display text-xl font-semibold">Alias Git</h2>
			<p class="text-sm text-muted">
				Un alias est un raccourci pour une commande Git : <code class="font-mono">co</code> pour
				<code class="font-mono">checkout</code>. Ceux qui lancent une commande du shell (commençant
				par <code class="font-mono">!</code>) sont refusés. Un alias désactivé reste ici mais n’est
				plus sur l’ordinateur.
			</p>
			<AliasList
				rules={GIT_ALIAS_RULES}
				aliases={config.aliases}
				onreplace={(name, alias) => edit(replaceAlias<GitConfig>(name, alias))}
				ontoggle={(name, enabled) => void edit(setAliasEnabled<GitConfig>(name, enabled))}
				onremove={(name) => void edit(removeAlias<GitConfig>(name))}
				fallbackFocus={() => (addForm ? addForm.focusName() : addAliasHeading?.focus())}
			/>
			<h3
				bind:this={addAliasHeading}
				tabindex="-1"
				id="{uid}-add-alias"
				class="font-display text-lg font-semibold"
			>
				Ajouter un alias Git
			</h3>
			{#if isFull(config, GIT_ALIASES_MAX)}
				<p class="text-sm text-muted">
					La configuration contient déjà le nombre maximal d’alias Git : supprimez-en un pour en
					ajouter.
				</p>
			{:else}
				<AliasForm
					bind:this={addForm}
					rules={GIT_ALIAS_RULES}
					labelledby="{uid}-add-alias"
					takenNames={config.aliases.map(({ name }) => name)}
					submitLabel="Ajouter"
					onsubmit={(alias) => edit(addAlias<GitConfig>(alias, GIT_ALIASES_MAX))}
				/>
			{/if}
			<h3
				bind:this={systemAliasHeading}
				tabindex="-1"
				id="{uid}-system-aliases"
				class="font-display text-lg font-semibold"
			>
				Alias présents ailleurs, non gérés
			</h3>
			<p class="text-sm text-muted">
				« Reprendre » ajoute l’alias à la configuration, activé, et commente sa ligne d’origine
				(préfixe <code class="font-mono">#hubergamof-off#</code>) : désactivé ici, il disparaît
				vraiment ; supprimé de la configuration, il retrouve sa ligne. Seuls
				<code class="font-mono">~/.gitconfig</code> et
				<code class="font-mono">~/.config/git/config</code> sont modifiés.
			</p>
			<SystemAliasList
				rules={GIT_ALIAS_RULES}
				aliases={unmanagedAliases}
				full={isFull(config, GIT_ALIASES_MAX)}
				describeSource={(source) => source}
				refuse={refuseOtherFile}
				unreadable={deck.git.gitAvailable === false
					? 'Non lu : le programme git est introuvable.'
					: undefined}
				onadopt={adopt}
				fallbackFocus={() => systemAliasHeading?.focus()}
			/>
		</section>

		<section aria-labelledby="{uid}-folders" class="flex flex-col gap-4 card">
			<h2
				bind:this={foldersHeading}
				tabindex="-1"
				id="{uid}-folders"
				class="font-display text-xl font-semibold"
			>
				Identités par dossier
			</h2>
			<p class="text-sm text-muted">
				Dans les dépôts d’un dossier (et de ses sous-dossiers), Git utilise ce nom, cette adresse et
				cette clé à la place des généraux : par exemple une adresse professionnelle sous
				<code class="font-mono">~/travail</code>. Un dossier est écrit sous votre dossier personnel
				et ne passe pas par un lien.
			</p>
			<GitFolderList
				rules={config.folderIdentities}
				onreplace={(folder, rule) => edit(replaceFolderIdentity(folder, rule))}
				ontoggle={(folder, enabled) => void edit(setFolderIdentityEnabled(folder, enabled))}
				onremove={(folder) => void edit(removeFolderIdentity(folder))}
				fallbackFocus={() =>
					addFolderForm ? addFolderForm.focusFolder() : foldersHeading?.focus()}
			/>
			<h3 id="{uid}-add-folder" class="font-display text-lg font-semibold">
				Ajouter une identité par dossier
			</h3>
			{#if isFolderListFull(config)}
				<p class="text-sm text-muted">
					La configuration contient déjà le nombre maximal d’identités par dossier : supprimez-en
					une pour en ajouter.
				</p>
			{:else}
				<GitFolderForm
					bind:this={addFolderForm}
					labelledby="{uid}-add-folder"
					takenFolders={config.folderIdentities.map(({ folder }) => folder)}
					submitLabel="Ajouter"
					onsubmit={(rule) => edit(addFolderIdentity(rule))}
				/>
			{/if}
		</section>

		<section aria-labelledby="{uid}-found" class="flex flex-col gap-4 card">
			<h2 id="{uid}-found" class="font-display text-xl font-semibold">
				Valeurs déjà présentes ailleurs
			</h2>
			<p class="text-sm text-muted">
				L’identité, la signature et les réglages que Git trouve dans d’autres fichiers (<code
					class="font-mono">~/.gitconfig</code
				>, <code class="font-mono">/etc/gitconfig</code>…), lus quand cette page s’ouvre. Ce que
				vous enregistrez ici les remplace.
			</p>
			<GitSystemEntries entries={otherEntries} unreadable={deck.git.gitAvailable === false} />
		</section>
	{/if}
</div>
