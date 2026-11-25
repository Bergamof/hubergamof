<!--
@component
The identity of the repositories under a folder, to add one or to change one: the folder (under the
home folder), the name, the address and the signing key that replace the global ones there, and
whether the rule is on the computer. A value left empty keeps the global one. The fields are checked
as the manager checks them; what is wrong is said under the field once the form was sent, and the
focus goes to the first one.
-->
<script lang="ts">
	import { onMount, tick } from 'svelte';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import {
		hasFolderIdentityErrors,
		trimValue,
		validateFolderIdentity
	} from '#lib/deck/git-validation.js';
	import type { GitFolderIdentity } from '#lib/generated/index.js';
	import FormCell from './FormCell.svelte';
	import FormGrid from './FormGrid.svelte';
	import Switch from './Switch.svelte';
	import TextField from './TextField.svelte';

	interface Props {
		/** The identity being changed; none to add one. */
		initial?: GitFolderIdentity;
		/** The folders of the other identities. */
		takenFolders: readonly string[];
		submitLabel: string;
		/** Saves the identity; the form is emptied (and `ondone` called) once it was. */
		onsubmit: (rule: GitFolderIdentity) => Promise<EditOutcome>;
		/** The change was saved. */
		ondone?: () => void;
		/** The change is given up (only when changing an identity). */
		oncancel?: () => void;
		/** Id of what names the form (a heading). */
		labelledby?: string;
	}

	let { initial, takenFolders, submitLabel, onsubmit, ondone, oncancel, labelledby }: Props =
		$props();

	const BLANK: GitFolderIdentity = {
		folder: '',
		name: '',
		email: '',
		signingKey: '',
		enabled: true
	};

	// The form starts from the identity it was opened for and keeps what is typed.
	// svelte-ignore state_referenced_locally
	let draft: GitFolderIdentity = $state({ ...(initial ?? BLANK) });
	let submitted = $state(false);
	let folderField: ReturnType<typeof TextField> | undefined = $state();
	let nameField: ReturnType<typeof TextField> | undefined = $state();
	let emailField: ReturnType<typeof TextField> | undefined = $state();
	let keyField: ReturnType<typeof TextField> | undefined = $state();

	function trimmed(rule: GitFolderIdentity): GitFolderIdentity {
		return {
			...rule,
			folder: trimValue(rule.folder),
			name: trimValue(rule.name),
			email: trimValue(rule.email),
			signingKey: trimValue(rule.signingKey)
		};
	}

	const errors = $derived(submitted ? validateFolderIdentity(trimmed(draft), takenFolders) : {});

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		submitted = true;
		const rule = trimmed(draft);
		const found = validateFolderIdentity(rule, takenFolders);
		if (hasFolderIdentityErrors(found)) {
			await tick();
			// Nothing is set: the person starts by typing an address.
			const target =
				(found.folder && folderField) ||
				(found.name && nameField) ||
				(found.email && emailField) ||
				(found.signingKey && keyField) ||
				emailField;
			target?.focus();
			return;
		}
		const { saved } = await onsubmit(rule);
		if (!saved) return;
		draft = { ...(initial ?? BLANK) };
		submitted = false;
		ondone?.();
		// Adding: ready for the next identity. Changing: the owner of the form decides where the focus goes.
		if (!initial) focusFolder();
	}

	/** Puts the focus in the folder field. */
	export function focusFolder() {
		folderField?.focus();
	}

	// Changing an identity: the form opens on its first field.
	onMount(() => {
		if (initial) focusFolder();
	});
	const uid = $props.id();
</script>

<FormGrid {labelledby} onsubmit={submit}>
	<FormCell span={12}>
		<TextField
			bind:this={folderField}
			label="Dossier"
			mono
			value={draft.folder}
			oninput={(folder) => (draft.folder = folder)}
			hint="Sous votre dossier personnel. Tous les dépôts qu’il contient sont concernés."
			error={errors.folder}
			placeholder="~/travail"
			maxlength={512}
		/>
	</FormCell>
	<FormCell span={4}>
		<TextField
			bind:this={nameField}
			label="Nom"
			value={draft.name}
			oninput={(name) => (draft.name = name)}
			placeholder="Valeur générale"
			error={errors.name}
			autocomplete="name"
			maxlength={512}
		/>
	</FormCell>
	<FormCell span={4}>
		<TextField
			bind:this={emailField}
			label="Adresse e-mail"
			type="email"
			value={draft.email}
			oninput={(email) => (draft.email = email)}
			placeholder="Valeur générale"
			error={errors.email}
			autocomplete="email"
			maxlength={512}
		/>
	</FormCell>
	<FormCell span={4}>
		<TextField
			bind:this={keyField}
			label="Clé de signature"
			mono
			value={draft.signingKey}
			oninput={(signingKey) => (draft.signingKey = signingKey)}
			placeholder="Valeur générale"
			error={errors.signingKey}
			maxlength={512}
		/>
	</FormCell>
	<FormCell>
		<p id="{uid}-empty" class="text-sm text-muted">
			Un champ laissé vide garde la valeur générale de Git.
		</p>
	</FormCell>
	{#if errors.empty}
		<FormCell>
			<p class="text-sm text-danger" role="alert">{errors.empty}</p>
		</FormCell>
	{/if}
	<FormCell>
		<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
			<Switch checked={draft.enabled} onchange={(checked) => (draft.enabled = checked)}>
				Activée
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
