<!--
@component
Who the commits are made as (`user.name`, `user.email`). Both may stay empty: Git keeps what it
had. The form is sent as a whole; what is wrong is said under the field once it was, and the focus
goes to the first one.
-->
<script lang="ts">
	import { FormDraft } from '#lib/deck/form-draft.svelte.js';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import {
		GIT_INPUT_MAX,
		trimValue,
		userEmailError,
		userNameError
	} from '#lib/deck/git-validation.js';
	import type { GitIdentity } from '#lib/generated/index.js';
	import FormFooter from './FormFooter.svelte';
	import FormCell from './FormCell.svelte';
	import FormGrid from './FormGrid.svelte';
	import TextField from './TextField.svelte';

	interface Props {
		/** The saved identity. */
		identity: GitIdentity;
		onsubmit: (identity: GitIdentity) => Promise<EditOutcome>;
		/** Id of what names the form (a heading). */
		labelledby?: string;
	}

	let { identity, onsubmit, labelledby }: Props = $props();

	const form = new FormDraft<GitIdentity>(() => identity);
	let nameField: ReturnType<typeof TextField> | undefined = $state();
	let emailField: ReturnType<typeof TextField> | undefined = $state();

	const trimmed = (values: GitIdentity): GitIdentity => ({
		name: trimValue(values.name),
		email: trimValue(values.email)
	});
	const errors = $derived(
		form.submitted
			? {
					name: userNameError(trimmed(form.shown).name),
					email: userEmailError(trimmed(form.shown).email)
				}
			: {}
	);

	const submit = (event: SubmitEvent) =>
		form.submit(
			event,
			trimmed,
			({ name, email }) =>
				userNameError(name)
					? () => nameField?.focus()
					: userEmailError(email)
						? () => emailField?.focus()
						: undefined,
			onsubmit
		);
</script>

<FormGrid {labelledby} onsubmit={submit}>
	<FormCell span={6}>
		<TextField
			bind:this={nameField}
			label="Nom"
			value={form.shown.name}
			oninput={(name) => form.edit({ name })}
			hint="Le nom des auteurs de vos commits (user.name)."
			error={errors.name}
			autocomplete="name"
			maxlength={GIT_INPUT_MAX}
		/>
	</FormCell>
	<FormCell span={6}>
		<TextField
			bind:this={emailField}
			label="Adresse e-mail"
			type="email"
			value={form.shown.email}
			oninput={(email) => form.edit({ email })}
			hint="L’adresse de vos commits (user.email)."
			error={errors.email}
			autocomplete="email"
			maxlength={GIT_INPUT_MAX}
		/>
	</FormCell>
	<FormCell>
		<FormFooter dirty={form.dirty} submitLabel="Enregistrer l’identité" />
	</FormCell>
</FormGrid>
