<!--
@component
The signature of the commits (`commit.gpgsign`, `gpg.format`, `user.signingkey`). While the
signature is off nothing of it is written on the computer. The form is sent as a whole.
-->
<script lang="ts">
	import { FormDraft } from '#lib/deck/form-draft.svelte.js';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import { GIT_INPUT_MAX, signingKeyError, trimValue } from '#lib/deck/git-validation.js';
	import type { GitSigning, GitSigningFormat } from '#lib/generated/index.js';
	import FormFooter from './FormFooter.svelte';
	import SegmentedControl from './SegmentedControl.svelte';
	import Switch from './Switch.svelte';
	import TextField from './TextField.svelte';

	interface Props {
		/** The saved signature. */
		signing: GitSigning;
		onsubmit: (signing: GitSigning) => Promise<EditOutcome>;
		/** Id of what names the form (a heading). */
		labelledby?: string;
	}

	let { signing, onsubmit, labelledby }: Props = $props();

	const FORMATS: { value: GitSigningFormat; label: string }[] = [
		{ value: 'openPgp', label: 'OpenPGP (GnuPG)' },
		{ value: 'ssh', label: 'Clé SSH' },
		{ value: 'x509', label: 'Certificat X.509' }
	];

	const KEY_HINTS: Record<GitSigningFormat, string> = {
		openPgp: 'L’identifiant de la clé GnuPG ; vide, Git choisit d’après votre adresse.',
		ssh: 'Le chemin de la clé publique (~/.ssh/id_ed25519.pub) ou « key:: » suivi de la clé.',
		x509: 'L’identifiant du certificat ; vide, Git choisit d’après votre adresse.'
	};

	const form = new FormDraft<GitSigning>(() => signing);
	let keyField: ReturnType<typeof TextField> | undefined = $state();

	const keyProblem = $derived(
		form.submitted ? signingKeyError(trimValue(form.shown.key)) : undefined
	);

	const submit = (event: SubmitEvent) =>
		form.submit(
			event,
			(values) => ({ ...values, key: trimValue(values.key) }),
			({ key }) => (signingKeyError(key) ? () => keyField?.focus() : undefined),
			onsubmit
		);
</script>

<form novalidate aria-labelledby={labelledby} class="flex flex-col gap-3" onsubmit={submit}>
	<Switch checked={form.shown.enabled} onchange={(enabled) => form.edit({ enabled })}>
		Signer mes commits
	</Switch>
	{#if form.shown.enabled}
		<SegmentedControl
			label="Format de la signature"
			options={FORMATS}
			value={form.shown.format}
			onchange={(format) => form.edit({ format })}
		/>
		<TextField
			bind:this={keyField}
			label="Clé de signature"
			mono
			value={form.shown.key}
			oninput={(key) => form.edit({ key })}
			hint={KEY_HINTS[form.shown.format]}
			error={keyProblem}
			maxlength={GIT_INPUT_MAX}
		/>
	{/if}
	<FormFooter dirty={form.dirty} submitLabel="Enregistrer la signature" />
</form>
