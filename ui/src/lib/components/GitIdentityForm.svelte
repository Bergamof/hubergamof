<!--
@component
Who the commits are made as (`user.name`, `user.email`). Both may stay empty: Git keeps what it
had. The form is sent as a whole; what is wrong is said under the field once it was, and the focus
goes to the first one.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import { trimValue, userEmailError, userNameError } from '#lib/deck/git-validation.js';
	import type { GitIdentity } from '#lib/generated/index.js';
	import FormFooter from './FormFooter.svelte';
	import TextField from './TextField.svelte';

	interface Props {
		/** The saved identity. */
		identity: GitIdentity;
		onsubmit: (identity: GitIdentity) => Promise<EditOutcome>;
		/** Id of what names the form (a heading). */
		labelledby?: string;
	}

	let { identity, onsubmit, labelledby }: Props = $props();

	/** What was typed since the last save; none: the saved identity is shown. */
	let draft: GitIdentity | undefined = $state();
	let submitted = $state(false);
	let nameField: ReturnType<typeof TextField> | undefined = $state();
	let emailField: ReturnType<typeof TextField> | undefined = $state();

	const shown = $derived(draft ?? identity);
	const errors = $derived(
		submitted
			? {
					name: userNameError(trimValue(shown.name)),
					email: userEmailError(trimValue(shown.email))
				}
			: {}
	);

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		submitted = true;
		const sent = { name: trimValue(shown.name), email: trimValue(shown.email) };
		const nameProblem = userNameError(sent.name);
		const emailProblem = userEmailError(sent.email);
		if (nameProblem || emailProblem) {
			await tick();
			(nameProblem ? nameField : emailField)?.focus();
			return;
		}
		const { saved } = await onsubmit(sent);
		if (!saved) return;
		draft = undefined;
		submitted = false;
	}
</script>

<form novalidate aria-labelledby={labelledby} class="grid gap-3 sm:grid-cols-2" onsubmit={submit}>
	<TextField
		bind:this={nameField}
		label="Nom"
		value={shown.name}
		oninput={(name) => (draft = { ...shown, name })}
		hint="Le nom des auteurs de vos commits (user.name)."
		error={errors.name}
		autocomplete="name"
		maxlength={512}
	/>
	<TextField
		bind:this={emailField}
		label="Adresse e-mail"
		type="email"
		value={shown.email}
		oninput={(email) => (draft = { ...shown, email })}
		hint="L’adresse de vos commits (user.email)."
		error={errors.email}
		autocomplete="email"
		maxlength={512}
	/>
	<div class="sm:col-span-2">
		<FormFooter dirty={draft !== undefined} submitLabel="Enregistrer l’identité" />
	</div>
</form>
