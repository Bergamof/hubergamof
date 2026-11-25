<!--
@component
A host the person connects to by a short name, to add one or to change one: its name, the machine it
stands for, the account, the port, the private key, the machines to go through, whether the agent is
forwarded, its other options and whether the host is on the computer. A value left empty is left to `ssh`. The fields are
checked as the manager checks them; what is wrong is said under the field once the form was sent, and
the focus goes to the first one.
-->
<script lang="ts">
	import { onMount, tick } from 'svelte';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import { trimValue } from '#lib/deck/git-validation.js';
	import {
		BLANK_HOST,
		SSH_FOLDER,
		SSH_INPUT_MAX,
		draftOf,
		hasHostErrors,
		hostOf,
		validateHost,
		type HostDraft
	} from '#lib/deck/ssh-validation.js';
	import type { SshHost } from '#lib/generated/index.js';
	import HostOptionsEditor from './HostOptionsEditor.svelte';
	import Switch from './Switch.svelte';
	import FormCell from './FormCell.svelte';
	import FormGrid from './FormGrid.svelte';
	import TextField from './TextField.svelte';

	interface Props {
		/** The host being changed; none to add one. */
		initial?: SshHost;
		/** The aliases of the other hosts. */
		takenAliases: readonly string[];
		submitLabel: string;
		/** Saves the host; the form is emptied (and `ondone` called) once it was. */
		onsubmit: (host: SshHost) => Promise<EditOutcome>;
		/** The change was saved. */
		ondone?: () => void;
		/** The change is given up (only when changing a host). */
		oncancel?: () => void;
		/** Id of what names the form (a heading). */
		labelledby?: string;
		/** What names the form when there is no heading for it (a form that changes a host). */
		label?: string;
	}

	let { initial, takenAliases, submitLabel, onsubmit, ondone, oncancel, labelledby, label }: Props =
		$props();

	const uid = $props.id();

	// The form starts from the host it was opened for and keeps what is typed.
	// svelte-ignore state_referenced_locally
	let draft: HostDraft = $state(initial ? draftOf(initial) : { ...BLANK_HOST });
	let submitted = $state(false);
	let optionsDetails: HTMLDetailsElement | undefined = $state();
	let optionsEditor: ReturnType<typeof HostOptionsEditor> | undefined = $state();
	let aliasField: ReturnType<typeof TextField> | undefined = $state();
	let hostNameField: ReturnType<typeof TextField> | undefined = $state();
	let userField: ReturnType<typeof TextField> | undefined = $state();
	let portField: ReturnType<typeof TextField> | undefined = $state();
	let keyField: ReturnType<typeof TextField> | undefined = $state();
	let jumpField: ReturnType<typeof TextField> | undefined = $state();

	function trimmed(values: HostDraft): HostDraft {
		return {
			...values,
			alias: trimValue(values.alias),
			hostName: trimValue(values.hostName),
			user: trimValue(values.user),
			port: trimValue(values.port),
			identityFile: trimValue(values.identityFile),
			proxyJump: trimValue(values.proxyJump)
		};
	}

	const errors = $derived(submitted ? validateHost(trimmed(draft), takenAliases) : {});

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		submitted = true;
		// An option typed but not added is added now, or says what is wrong: it is not lost.
		if (optionsEditor && !optionsEditor.commit()) return;
		const values = trimmed(draft);
		const found = validateHost(values, takenAliases);
		if (hasHostErrors(found)) {
			await tick();
			const target = [
				[found.alias, aliasField],
				[found.hostName, hostNameField],
				[found.user, userField],
				[found.port, portField],
				[found.identityFile, keyField],
				[found.proxyJump, jumpField]
			].find(([problem]) => problem)?.[1] as ReturnType<typeof TextField> | undefined;
			target?.focus();
			return;
		}
		const { saved } = await onsubmit(hostOf(values));
		if (!saved) return;
		draft = initial ? draftOf(initial) : { ...BLANK_HOST };
		submitted = false;
		ondone?.();
		// Adding: ready for the next host. Changing: the owner of the form decides where the focus goes.
		if (!initial) focusAlias();
	}

	/** Puts the focus in the name field. */
	export function focusAlias() {
		aliasField?.focus();
	}

	// Changing a host: the form opens on its first field, and shows the options the host has. Only
	// then: afterwards the browser keeps what the person chose (an attribute kept in step with the
	// draft would shut the options as soon as one is added).
	onMount(() => {
		if (initial) focusAlias();
		if (initial && initial.options.length > 0 && optionsDetails) optionsDetails.open = true;
	});
</script>

<FormGrid {label} {labelledby} onsubmit={submit}>
	<FormCell span={4}>
		<TextField
			bind:this={aliasField}
			label="Nom du serveur"
			mono
			placeholder="prod"
			value={draft.alias}
			oninput={(alias) => (draft.alias = alias)}
			hint="Ce qu’on tape après ssh."
			error={errors.alias}
			maxlength={SSH_INPUT_MAX}
		/>
	</FormCell>
	<FormCell span={5}>
		<TextField
			bind:this={hostNameField}
			label="Adresse de la machine"
			mono
			placeholder="Vide : le nom du serveur"
			value={draft.hostName}
			oninput={(hostName) => (draft.hostName = hostName)}
			error={errors.hostName}
			maxlength={SSH_INPUT_MAX}
		/>
	</FormCell>
	<FormCell span={3}>
		<TextField
			bind:this={portField}
			label="Port"
			mono
			placeholder="22"
			inputmode="numeric"
			value={draft.port}
			oninput={(port) => (draft.port = port)}
			error={errors.port}
			maxlength={8}
		/>
	</FormCell>
	<FormCell span={4}>
		<TextField
			bind:this={userField}
			label="Utilisateur"
			mono
			placeholder="Votre compte actuel"
			value={draft.user}
			oninput={(user) => (draft.user = user)}
			error={errors.user}
			autocomplete="off"
			maxlength={SSH_INPUT_MAX}
		/>
	</FormCell>
	<FormCell span={8}>
		<TextField
			bind:this={keyField}
			label="Clé privée"
			mono
			placeholder="{SSH_FOLDER}id_ed25519"
			value={draft.identityFile}
			oninput={(identityFile) => (draft.identityFile = identityFile)}
			hint="Dans {SSH_FOLDER}. Vide : ssh essaie les clés habituelles."
			error={errors.identityFile}
			maxlength={SSH_INPUT_MAX}
		/>
	</FormCell>
	<FormCell span={8}>
		<TextField
			bind:this={jumpField}
			label="Rebonds"
			mono
			placeholder="bastion ou moi@bastion:2222"
			value={draft.proxyJump}
			oninput={(proxyJump) => (draft.proxyJump = proxyJump)}
			hint="Les machines à traverser, séparées par une virgule. Vide : connexion directe."
			error={errors.proxyJump}
			maxlength={SSH_INPUT_MAX}
		/>
	</FormCell>
	<FormCell span={4}>
		<!-- The risk is said where it applies: when the agent is forwarded. -->
		<div class="flex h-full items-end pb-1">
			<Switch
				checked={draft.forwardAgent}
				onchange={(checked) => (draft.forwardAgent = checked)}
				describedby={draft.forwardAgent ? `${uid}-agent` : undefined}
			>
				Transmettre l’agent SSH
			</Switch>
		</div>
	</FormCell>
	{#if draft.forwardAgent}
		<FormCell>
			<p id="{uid}-agent" class="rounded-lg bg-warning-soft px-3 py-2 text-sm text-warning">
				Attention : qui contrôle la machine distante peut utiliser vos clés pendant la connexion. À
				réserver aux serveurs de confiance.
			</p>
		</FormCell>
	{/if}
	<FormCell>
		<details bind:this={optionsDetails}>
			<summary id="{uid}-options" class="cursor-pointer py-1 text-sm font-medium">
				Options avancées{draft.options.length > 0 ? ` (${draft.options.length})` : ''}
			</summary>
			<div class="mt-3">
				<HostOptionsEditor
					bind:this={optionsEditor}
					options={draft.options}
					host={initial?.alias}
					labelledby="{uid}-options"
					onchange={(options) => (draft.options = options)}
				/>
			</div>
		</details>
	</FormCell>
	<FormCell>
		<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
			<Switch checked={draft.enabled} onchange={(checked) => (draft.enabled = checked)}>
				Activé
			</Switch>
			<div class="ml-auto flex gap-2">
				{#if oncancel}
					<button type="button" class="secondary-button" onclick={oncancel}>Annuler</button>
				{/if}
				<button type="submit" class="primary-button">{submitLabel}</button>
			</div>
		</div>
	</FormCell>
</FormGrid>
