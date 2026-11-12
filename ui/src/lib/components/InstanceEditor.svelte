<!--
@component
Label and settings form of a module instance, saved as it is edited: a choice at once, a text
once it is finished (field left, Enter, or a pause in the typing of {@link TYPING_PAUSE_MS}).
Escape in a text field gives back its saved value. A field refused stays as typed and is not
saved; the valid ones are. Its error shows once the field is left or confirmed, never at a
pause in the typing, where the text is often unfinished. A refusal of the manager keeps what was typed and
says why. When the instance changes elsewhere (another tab), the fields left untouched follow;
one being changed keeps what is typed, saved over the other change once finished.
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
	import type { InstanceChanges } from '#lib/deck/config-edits.js';
	import type { EditOutcome } from '#lib/deck/deck-state.svelte.js';
	import type { Problem } from '#lib/deck/problems.js';
	import { debounce } from '#lib/deck/debounce.js';
	import { moduleName } from '#lib/deck/display.js';
	import { cloneJson, sameJson } from '#lib/deck/json.js';
	import type { ManagerApi } from '#lib/api/manager-api.js';
	import {
		parseDraft,
		parseLabel,
		sameDraftValue,
		toDraft,
		type SettingsDraft,
		type SettingsErrors
	} from '#lib/deck/settings-form.js';
	import ProblemAlert from './ProblemAlert.svelte';
	import SettingInput from './SettingInput.svelte';

	interface Props {
		instance: ModuleInstance;
		module: CatalogModule | undefined;
		/** Device the instance belongs to, and the client storing the images of its settings. */
		serial: string;
		api: ManagerApi;
		onapply: (changes: InstanceChanges) => Promise<EditOutcome>;
	}

	let { instance, module, serial, api, onapply }: Props = $props();

	/** Longest label: it is drawn at the bottom of a key, where little more fits. */
	const MAX_LABEL_LENGTH = 40;
	/** Pause in the typing after which a text counts as finished, and is saved. */
	const TYPING_PAUSE_MS = 1000;

	const uid = $props.id();
	const fields = $derived(module?.settings ?? []);

	/** Plain copy of the instance as displayed now, detached from the reactive configuration. */
	function snapshot(): ModuleInstance {
		return cloneJson(instance);
	}

	/** Instance the form was filled from. */
	const initial = snapshot();
	/** The instance as last saved by this form, or received from elsewhere. */
	let base = $state(initial);
	let label = $state(labelOf(initial));
	let draft: SettingsDraft = $state(draftOf(initial));
	/** Errors shown: those of the fields left or confirmed (see {@link save}). */
	let errors: SettingsErrors = $state({});
	let saving = $state(false);
	/** A save was asked while saving: done next, with whatever is typed by then. */
	let saveAgain: { showErrors: boolean } | undefined;
	/** Last outcome said by the status: kept while typing, so it is not read again and again. */
	let notice = $state('');
	let refusal: Problem | undefined = $state();

	const saveAfterPause = debounce(() => void save({ showErrors: false }), TYPING_PAUSE_MS);

	const parsed = $derived(parseDraft(fields, draft, base.settings));
	/** While saving, the displayed instance already carries the change: not « elsewhere ». */
	const changedElsewhere = $derived(
		!saving && (base.label !== instance.label || !sameJson(base.settings, instance.settings))
	);
	/** Names the fields refused, so the status says which ones are not saved. */
	const invalidFields = $derived(
		fields.filter((field) => errors[field.key] !== undefined).map((field) => field.label)
	);

	// Keeps the form in step with the configuration, which may change from another tab at any
	// time.
	$effect(() => {
		if (changedElsewhere) untrack(rebase);
	});

	// A text typed just before leaving (another key selected, tab closed) is still saved.
	$effect(() => {
		const leave = () => saveAfterPause.flushPending();
		window.addEventListener('pagehide', leave);
		return () => {
			window.removeEventListener('pagehide', leave);
			leave();
		};
	});

	function labelOf(source: ModuleInstance): string {
		return source.label ?? '';
	}

	function draftOf(source: ModuleInstance): SettingsDraft {
		return toDraft(fields, source.settings);
	}

	/**
	 * Takes the displayed instance as the saved one, and shows its values in the fields the user
	 * has not changed; a field changed keeps what is typed. An unchanged list of files is left
	 * as it is: a new list would close the file being edited.
	 */
	function rebase() {
		const before = draftOf(base);
		const beforeLabel = labelOf(base);
		base = snapshot();
		const after = draftOf(base);
		if (label === beforeLabel) label = labelOf(base);
		for (const [key, value] of Object.entries(after)) {
			const untouched = sameDraftValue(draft[key], before[key]);
			if (untouched && !sameDraftValue(draft[key], value)) draft[key] = value;
		}
	}

	/** A keystroke: saved once the typing pauses, unless the field is left first. */
	function typed() {
		saveAfterPause();
	}

	/** A change is finished: saved now, its errors shown. */
	function commit() {
		saveAfterPause.cancel();
		void save({ showErrors: true });
	}

	async function reverted() {
		saveAfterPause.cancel();
		await save({ showErrors: true });
		if (!refusal) notice = 'Valeur enregistrée rétablie.';
	}

	function revertLabel() {
		label = labelOf(base);
		void reverted();
	}

	function revertSetting(key: string) {
		const value = draftOf(base)[key];
		if (value !== undefined) draft[key] = value;
		void reverted();
	}

	/**
	 * The errors to show: all of them once a change is finished; at a pause in the typing, only
	 * those already shown that still apply (a field being corrected loses its error).
	 */
	function errorsToShow(showErrors: boolean): SettingsErrors {
		if (showErrors) return parsed.errors;
		return Object.fromEntries(
			Object.entries(parsed.errors).filter(([key]) => errors[key] !== undefined)
		);
	}

	/**
	 * Saves what the form holds, leaving out the fields refused. One save at a time: one asked
	 * meanwhile is done right after, with the latest values.
	 */
	async function save({ showErrors }: { showErrors: boolean }) {
		if (saving) {
			saveAgain = { showErrors: showErrors || (saveAgain?.showErrors ?? false) };
			return;
		}
		errors = errorsToShow(showErrors);
		const changes: InstanceChanges = { label: parseLabel(label), settings: parsed.settings };
		if (changes.label === (base.label ?? null) && sameJson(changes.settings, base.settings)) {
			// Back to the saved values (Escape, or the same text typed again): nothing to send.
			refusal = undefined;
			return;
		}
		refusal = undefined;
		saving = true;
		try {
			const outcome = await onapply(changes);
			if (outcome.saved) {
				rebase();
				notice = 'Réglages enregistrés.';
			} else {
				refusal = outcome.problem;
				notice = '';
			}
		} finally {
			saving = false;
		}
		const next = saveAgain;
		saveAgain = undefined;
		if (next) await save(next);
	}
</script>

<form
	class="flex flex-col gap-4"
	aria-labelledby="{uid}-title"
	aria-describedby="{uid}-help"
	onsubmit={(event) => event.preventDefault()}
	novalidate
>
	<div class="flex flex-col gap-1">
		<h3 id="{uid}-title" class="font-semibold">Réglages</h3>
		<p id="{uid}-help" class="text-sm text-muted">
			Enregistrés automatiquement : un choix tout de suite, un texte à la fin de la saisie (pause,
			Tab ou Entrée). Échap rétablit la valeur enregistrée d’un champ.
		</p>
	</div>

	<div class="flex flex-col gap-1">
		<label class="font-medium" for="{uid}-label">Libellé</label>
		<input
			id="{uid}-label"
			type="text"
			class="field"
			placeholder={moduleName(instance.kind, module)}
			maxlength={MAX_LABEL_LENGTH}
			bind:value={label}
			oninput={typed}
			onchange={commit}
			onkeydown={(event) => {
				if (event.isComposing) return;
				if (event.key === 'Enter') commit();
				else if (event.key === 'Escape') revertLabel();
				else return;
				event.preventDefault();
			}}
			aria-describedby="{uid}-label-hint"
		/>
		<p id="{uid}-label-hint" class="text-sm text-muted">
			Vide : nom du module. {MAX_LABEL_LENGTH} caractères au plus.
		</p>
	</div>

	{#if !module}
		<p class="text-sm text-muted">
			Ce module n’est pas encore disponible dans cette version : seul son libellé est modifiable.
		</p>
	{/if}

	{#each fields as field (field.key)}
		<SettingInput
			{field}
			idPrefix={uid}
			bind:value={draft[field.key]}
			error={errors[field.key]}
			{serial}
			{api}
			moduleKind={instance.kind}
			defaultIcon={module?.settingIcons[field.key] ?? module?.icon}
			ontype={typed}
			oncommit={commit}
			onrevert={() => revertSetting(field.key)}
		/>
	{/each}

	{#if refusal}
		<ProblemAlert problem={refusal} lead="Réglages non enregistrés." />
	{/if}

	<p class="flex flex-wrap gap-x-2 text-sm text-muted">
		<!-- Only the outcome is read out: « Enregistrement… » would be cut at every pause. -->
		<span role="status">
			{#if invalidFields.length > 0}
				Non enregistré tant que ce n’est pas corrigé : {invalidFields.join(', ')}.
			{:else}
				{notice}
			{/if}
		</span>
		{#if saving}
			<span aria-hidden="true">Enregistrement…</span>
		{/if}
	</p>
</form>
