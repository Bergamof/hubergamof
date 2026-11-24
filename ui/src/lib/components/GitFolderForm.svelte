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
</script>

<form novalidate aria-labelledby={labelledby} class="grid gap-3 sm:grid-cols-2" onsubmit={submit}>
	<div class="sm:col-span-2">
		<TextField
			bind:this={folderField}
			label="Dossier"
			mono
			value={draft.folder}
			oninput={(folder) => (draft.folder = folder)}
			hint="Sous votre dossier personnel, par exemple ~/travail. Tous les dépôts qu’il contient sont concernés."
			error={errors.folder}
			placeholder="~/travail"
			maxlength={512}
		/>
	</div>
	<TextField
		bind:this={nameField}
		label="Nom"
		value={draft.name}
		oninput={(name) => (draft.name = name)}
		hint="Vide : le nom général."
		error={errors.name}
		autocomplete="name"
		maxlength={512}
	/>
	<TextField
		bind:this={emailField}
		label="Adresse e-mail"
		type="email"
		value={draft.email}
		oninput={(email) => (draft.email = email)}
		hint="Vide : l’adresse générale."
		error={errors.email}
		autocomplete="email"
		maxlength={512}
	/>
	<div class="sm:col-span-2">
		<TextField
			bind:this={keyField}
			label="Clé de signature"
			mono
			value={draft.signingKey}
			oninput={(signingKey) => (draft.signingKey = signingKey)}
			hint="Vide : la clé générale."
			error={errors.signingKey}
			maxlength={512}
		/>
	</div>
	{#if errors.empty}
		<p class="text-sm text-danger sm:col-span-2" role="alert">{errors.empty}</p>
	{/if}
	<div class="flex flex-wrap items-center gap-x-4 gap-y-2 sm:col-span-2">
		<Switch checked={draft.enabled} onchange={(checked) => (draft.enabled = checked)}>
			Activée
		</Switch>
		<div class="flex gap-2 sm:ml-auto">
			{#if oncancel}
				<button type="button" class="secondary-button" onclick={oncancel}>Annuler</button>
			{/if}
			<button type="submit" class="primary-button">{submitLabel}</button>
		</div>
	</div>
</form>
