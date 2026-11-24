<!--
@component
The signature of the commits (`commit.gpgsign`, `gpg.format`, `user.signingkey`). While the
signature is off nothing of it is written on the computer. The form is sent as a whole.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import { signingKeyError, trimValue } from '#lib/deck/git-validation.js';
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

	/** What was typed since the last save; none: the saved signature is shown. */
	let draft: GitSigning | undefined = $state();
	let submitted = $state(false);
	let keyField: ReturnType<typeof TextField> | undefined = $state();

	const shown = $derived(draft ?? signing);
	const keyProblem = $derived(submitted ? signingKeyError(trimValue(shown.key)) : undefined);

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		submitted = true;
		const sent = { ...shown, key: trimValue(shown.key) };
		if (signingKeyError(sent.key)) {
			await tick();
			keyField?.focus();
			return;
		}
		const { saved } = await onsubmit(sent);
		if (!saved) return;
		draft = undefined;
		submitted = false;
	}
</script>

<form novalidate aria-labelledby={labelledby} class="flex flex-col gap-3" onsubmit={submit}>
	<Switch checked={shown.enabled} onchange={(enabled) => (draft = { ...shown, enabled })}>
		Signer mes commits
	</Switch>
	{#if shown.enabled}
		<SegmentedControl
			label="Format de la signature"
			options={FORMATS}
			value={shown.format}
			onchange={(format) => (draft = { ...shown, format })}
		/>
		<TextField
			bind:this={keyField}
			label="Clé de signature"
			mono
			value={shown.key}
			oninput={(key) => (draft = { ...shown, key })}
			hint={KEY_HINTS[shown.format]}
			error={keyProblem}
			maxlength={512}
		/>
	{/if}
	<FormFooter dirty={draft !== undefined} submitLabel="Enregistrer la signature" />
</form>
