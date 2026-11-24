<!--
@component
The SSH hosts managed by HuBergamof, in the order they are written: each can be switched, changed or
removed (after a confirmation: the host leaves the computer with it). The focus stays where the
person works: back to « Modifier » after a change, on the neighbour (or `fallbackFocus` for the last
one) after a removal.
-->
<script lang="ts">
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import type { SettingsEdit } from '#lib/deck/advanced-edits.js';
	import { SSH_OPTION_ALIAS_RULES } from '#lib/deck/alias-rules.js';
	import type { SshHost } from '#lib/generated/index.js';
	import AdvancedSettings from './AdvancedSettings.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import SshHostForm from './SshHostForm.svelte';
	import Switch from './Switch.svelte';

	interface Props {
		hosts: readonly SshHost[];
		onreplace: (alias: string, host: SshHost) => Promise<EditOutcome>;
		ontoggle: (alias: string, enabled: boolean) => void;
		onremove: (alias: string) => void;
		/** Changes the options of the host called `alias`. */
		onoptions: (alias: string, change: SettingsEdit) => Promise<EditOutcome>;
		/** Where the focus goes once a host is removed. */
		fallbackFocus?: () => void;
	}

	let { hosts, onreplace, ontoggle, onremove, onoptions, fallbackFocus }: Props = $props();

	let list: HTMLUListElement | undefined = $state();
	/** The host whose form is open. */
	let editingAlias: string | undefined = $state();
	/** The host whose « Modifier » button takes the focus as soon as it is displayed. */
	let editFocusAlias: string | undefined = $state();
	/** The host waiting for the person to confirm its removal. */
	let removing: SshHost | undefined = $state();

	// A renamed host is a new row: the button exists once the list has the new alias, hence `hosts`.
	$effect(() => {
		void hosts;
		if (editFocusAlias === undefined) return;
		const button = rowOf(editFocusAlias)?.querySelector<HTMLButtonElement>('[data-edit]');
		if (!button) return;
		button.focus();
		editFocusAlias = undefined;
	});

	function closeEditor() {
		editFocusAlias = editingAlias;
		editingAlias = undefined;
	}

	async function replace(alias: string, changed: SshHost): Promise<EditOutcome> {
		const outcome = await onreplace(alias, changed);
		if (outcome.saved) {
			editFocusAlias = changed.alias;
			editingAlias = undefined;
		}
		return outcome;
	}

	/** The neighbour whose « Supprimer » button takes the focus once the confirmation is closed (none: the last one). */
	let afterRemoval: { neighbour: string | undefined } | undefined;

	function rowOf(alias: string): HTMLLIElement | undefined {
		return [...(list?.querySelectorAll<HTMLLIElement>(':scope > li[data-alias]') ?? [])].find(
			(item) => item.dataset.alias === alias
		);
	}

	function confirmRemoval() {
		if (removing) {
			const index = hosts.findIndex(({ alias }) => alias === removing?.alias);
			afterRemoval = { neighbour: (hosts[index + 1] ?? hosts[index - 1])?.alias };
			onremove(removing.alias);
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

	/** What the host stands for, in words. */
	function summary(host: SshHost): string {
		const target = `${host.user ? `${host.user}@` : ''}${host.hostName || host.alias}${host.port === null ? '' : `:${host.port}`}`;
		return [
			target,
			host.proxyJump && `via ${host.proxyJump}`,
			host.identityFile && `clé ${host.identityFile}`,
			host.forwardAgent && 'agent transmis'
		]
			.filter(Boolean)
			.join(' · ');
	}
</script>

{#if hosts.length === 0}
	<p class="text-sm text-muted">Aucun serveur géré pour l’instant.</p>
{:else}
	<ul bind:this={list} class="flex flex-col gap-2" aria-label="Serveurs SSH gérés">
		{#each hosts as host (host.alias)}
			<li data-alias={host.alias}>
				{#if editingAlias === host.alias}
					<div class="rounded-xl border border-ring bg-sunken p-3">
						<SshHostForm
							initial={host}
							takenAliases={hosts
								.filter(({ alias }) => alias !== host.alias)
								.map(({ alias }) => alias)}
							label="Modifier le serveur {host.alias}"
							submitLabel="Enregistrer"
							onsubmit={(changed) => replace(host.alias, changed)}
							oncancel={closeEditor}
						/>
					</div>
				{:else}
					<div
						class="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border border-line p-3"
					>
						<div class={['flex min-w-0 flex-1 basis-60 flex-col', !host.enabled && 'text-muted']}>
							<code class="font-mono font-semibold break-all">{host.alias}</code>
							<span class="text-sm break-all">{summary(host)}</span>
						</div>
						{#if !host.enabled}
							<span class="rounded-full border border-line px-2 py-0.5 text-sm text-muted">
								Désactivé
							</span>
						{/if}
						<Switch
							checked={host.enabled}
							label="Serveur {host.alias}"
							onchange={(enabled) => ontoggle(host.alias, enabled)}
						/>
						<div class="flex gap-2">
							<button
								type="button"
								data-edit
								class="secondary-button"
								aria-label="Modifier le serveur {host.alias}"
								onclick={() => (editingAlias = host.alias)}
							>
								Modifier
							</button>
							<button
								type="button"
								data-remove
								class="secondary-button"
								aria-label="Supprimer le serveur {host.alias}"
								onclick={() => (removing = host)}
							>
								Supprimer
							</button>
						</div>
						<details class="basis-full" data-options>
							<summary class="cursor-pointer text-sm font-medium">
								Options avancées{host.options.length > 0 ? ` (${host.options.length})` : ''}
								<span class="sr-only">du serveur {host.alias}</span>
							</summary>
							<div class="mt-3 flex flex-col gap-3">
								<p class="text-sm text-muted">
									Écrites telles quelles sous ce serveur : ssh vérifie qu’il les comprend avant
									qu’elles soient enregistrées sur l’ordinateur.
								</p>
								<AdvancedSettings
									settings={host.options}
									rules={SSH_OPTION_ALIAS_RULES}
									addLabel="Ajouter une option à {host.alias}"
									onchange={(change) => onoptions(host.alias, change)}
								/>
							</div>
						</details>
					</div>
				{/if}
			</li>
		{/each}
	</ul>
{/if}

<ConfirmDialog
	open={removing !== undefined}
	title="Supprimer le serveur ?"
	confirmLabel="Supprimer"
	onconfirm={confirmRemoval}
	oncancel={() => (removing = undefined)}
	onclosed={focusAfterRemoval}
>
	Le serveur <code class="font-mono font-semibold">{removing?.alias}</code> sera retiré de la configuration
	et de votre ordinateur.
</ConfirmDialog>
