<!--
@component
How Git behaves: the first branch of a new repository, what `git pull` does, the editor and where
the passwords are kept. What is left at « ne pas gérer » is left as it is on the computer. The
editor and the credential helper come from lists, since Git runs them. The form is sent as a whole.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import {
		CREDENTIAL_CACHE_DEFAULT,
		GIT_EDITORS,
		branchError,
		cacheTimeoutError,
		editorError,
		trimValue
	} from '#lib/deck/git-validation.js';
	import type { GitCredentialHelper, GitPullMode, GitSettings } from '#lib/generated/index.js';
	import FormFooter from './FormFooter.svelte';
	import FormCell from './FormCell.svelte';
	import FormGrid from './FormGrid.svelte';
	import SegmentedControl from './SegmentedControl.svelte';
	import SelectField from './SelectField.svelte';
	import TextField from './TextField.svelte';

	interface Props {
		/** The saved settings. */
		settings: GitSettings;
		onsubmit: (settings: GitSettings) => Promise<EditOutcome>;
		/** Id of what names the form (a heading). */
		labelledby?: string;
	}

	let { settings, onsubmit, labelledby }: Props = $props();

	const PULL_MODES: { value: GitPullMode; label: string }[] = [
		{ value: 'default', label: 'Choix de Git' },
		{ value: 'merge', label: 'Fusionner' },
		{ value: 'rebase', label: 'Rebaser' },
		{ value: 'fastForwardOnly', label: 'Avance rapide seulement' }
	];

	type HelperKind = GitCredentialHelper['kind'];
	const HELPERS: { value: HelperKind; label: string }[] = [
		{ value: 'none', label: 'Ne pas gérer' },
		{ value: 'cache', label: 'En mémoire, pour un temps limité (cache)' },
		{ value: 'store', label: 'Dans un fichier en clair (store)' },
		{ value: 'libsecret', label: 'Dans le trousseau du bureau (libsecret)' },
		{ value: 'manager', label: 'Git Credential Manager (manager)' }
	];

	/** The value of the editor drop-down for the program that stands for « another program ». */
	const OTHER_EDITOR = 'other';
	const EDITORS = [
		{ value: '', label: 'Ne pas gérer' },
		...GIT_EDITORS.map((editor) => ({ value: editor, label: editor })),
		{ value: OTHER_EDITOR, label: 'Un de ces éditeurs, par son chemin absolu' }
	];

	/** What was typed since the last save; none: the saved settings are shown. */
	let draft: GitSettings | undefined = $state();
	/** « Un autre programme » was chosen: the path is typed in its own field. */
	let otherEditor: boolean | undefined = $state();
	/** The duration of the cache as typed; none: the saved one. */
	let cacheText: string | undefined = $state();
	let submitted = $state(false);
	let branchField: ReturnType<typeof TextField> | undefined = $state();
	let editorField: ReturnType<typeof TextField> | undefined = $state();
	let cacheField: ReturnType<typeof TextField> | undefined = $state();

	const shown = $derived(draft ?? settings);
	const helperKind = $derived(shown.credentialHelper.kind);
	const isOtherEditor = $derived(
		otherEditor ?? (shown.editor !== '' && !GIT_EDITORS.includes(shown.editor))
	);
	const editorChoice = $derived(isOtherEditor ? OTHER_EDITOR : shown.editor);
	const savedCache = $derived(
		settings.credentialHelper.kind === 'cache'
			? settings.credentialHelper.timeoutSeconds
			: CREDENTIAL_CACHE_DEFAULT
	);
	const cacheValue = $derived(cacheText ?? String(savedCache));
	const dirty = $derived(
		draft !== undefined || otherEditor !== undefined || cacheText !== undefined
	);

	interface Problems {
		branch?: string;
		editor?: string;
		cache?: string;
	}

	function found(sent: GitSettings): Problems {
		return {
			branch: branchError(sent.defaultBranch),
			editor:
				isOtherEditor && sent.editor === ''
					? 'Saisissez le chemin du programme, ou choisissez « Ne pas gérer ».'
					: editorError(sent.editor),
			cache:
				sent.credentialHelper.kind === 'cache'
					? cacheTimeoutError(sent.credentialHelper.timeoutSeconds)
					: undefined
		};
	}

	/** The settings as they would be sent: trimmed, the helper with the duration typed. */
	function toSend(): GitSettings {
		const credentialHelper: GitCredentialHelper =
			helperKind === 'cache'
				? { kind: 'cache', timeoutSeconds: Number(trimValue(cacheValue)) }
				: { kind: helperKind };
		return {
			...shown,
			defaultBranch: trimValue(shown.defaultBranch),
			editor: trimValue(shown.editor),
			credentialHelper
		};
	}

	const errors: Problems = $derived(submitted ? found(toSend()) : {});

	function chooseEditor(choice: string) {
		otherEditor = choice === OTHER_EDITOR;
		draft = { ...shown, editor: choice === OTHER_EDITOR ? '' : choice };
	}

	function chooseHelper(kind: HelperKind) {
		draft = {
			...shown,
			credentialHelper:
				kind === 'cache' ? { kind, timeoutSeconds: savedCache } : ({ kind } as GitCredentialHelper)
		};
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		submitted = true;
		const sent = toSend();
		const problems = found(sent);
		if (problems.branch || problems.editor || problems.cache) {
			await tick();
			(problems.branch ? branchField : problems.editor ? editorField : cacheField)?.focus();
			return;
		}
		const { saved } = await onsubmit(sent);
		if (!saved) return;
		draft = undefined;
		otherEditor = undefined;
		cacheText = undefined;
		submitted = false;
	}
</script>

<FormGrid {labelledby} onsubmit={submit}>
	<FormCell span={6}>
		<TextField
			bind:this={branchField}
			label="Première branche d’un nouveau dépôt"
			mono
			placeholder="main"
			value={shown.defaultBranch}
			oninput={(defaultBranch) => (draft = { ...shown, defaultBranch })}
			hint="init.defaultBranch. Vide : Git garde son choix."
			error={errors.branch}
			maxlength={512}
		/>
	</FormCell>
	<FormCell span={6}>
		<div class="flex flex-col gap-1">
			<span class="text-sm font-medium">Que fait « git pull » ?</span>
			<SegmentedControl
				label="Que fait « git pull » ?"
				options={PULL_MODES}
				value={shown.pull}
				onchange={(pull) => (draft = { ...shown, pull })}
			/>
		</div>
	</FormCell>
	<FormCell span={6}>
		<SelectField
			label="Éditeur des messages de commit"
			options={EDITORS}
			value={editorChoice}
			onchange={chooseEditor}
			hint="core.editor : Git lance ce programme, la liste est donc fermée."
		/>
	</FormCell>
	{#if isOtherEditor}
		<FormCell span={6}>
			<TextField
				bind:this={editorField}
				label="Chemin du programme"
				mono
				value={shown.editor}
				oninput={(editor) => (draft = { ...shown, editor })}
				hint="Le chemin absolu du programme de vi, vim, nvim, nano, emacs, micro ou hx, sans argument. Vide : Git garde son éditeur."
				error={errors.editor}
				maxlength={512}
			/>
		</FormCell>
	{/if}
	<FormCell span={6}>
		<SelectField
			label="Mots de passe"
			options={HELPERS}
			value={helperKind}
			onchange={chooseHelper}
			hint="credential.helper : où Git garde les mots de passe qu’on lui a donnés."
			warning={helperKind === 'store'
				? 'Les mots de passe sont écrits en clair dans ~/.git-credentials.'
				: undefined}
		/>
	</FormCell>
	{#if helperKind === 'cache'}
		<FormCell span={6}>
			<TextField
				bind:this={cacheField}
				label="Durée en mémoire (secondes)"
				value={cacheValue}
				oninput={(text) => (cacheText = text)}
				inputmode="numeric"
				hint="De 30 s à 7 jours (604800 s)."
				error={errors.cache}
				maxlength={16}
			/>
		</FormCell>
	{/if}
	<FormCell>
		<FormFooter {dirty} submitLabel="Enregistrer les réglages" />
	</FormCell>
</FormGrid>
