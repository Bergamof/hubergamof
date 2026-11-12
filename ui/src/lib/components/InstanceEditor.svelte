<!--
@component
Label and settings form of a module instance, saved as it is edited: a choice at once, a text
once it is finished (field left, Enter, or a pause in the typing of {@link TYPING_PAUSE_MS}).
Escape in a text field gives back its saved value. A field refused stays as typed, with its
error, and is not saved; the valid ones are. A refusal of the manager keeps what was typed and
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
	/** Errors shown, from the last finished change (not at every keystroke). */
	let errors: SettingsErrors = $state({});
	let saving = $state(false);
	/** A change was finished while saving: saved next, with whatever is typed by then. */
	let saveAgain = false;
	let saved = $state(false);
	let refusal: Problem | undefined = $state();

	const saveAfterPause = debounce(() => void save(), TYPING_PAUSE_MS);

	const parsed = $derived(parseDraft(fields, draft, base.settings));
	/** While saving, the displayed instance already carries the change: not « elsewhere ». */
	const changedElsewhere = $derived(
		!saving && (base.label !== instance.label || !sameJson(base.settings, instance.settings))
	);
	const invalid = $derived(Object.keys(errors).length > 0);

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
		saved = false;
		saveAfterPause();
	}

	/** A change is finished: saved now. */
	function commit() {
		saveAfterPause.flush();
	}

	function revertLabel() {
		label = labelOf(base);
		commit();
	}

	function revertSetting(key: string) {
		const value = draftOf(base)[key];
		if (value !== undefined) draft[key] = value;
		commit();
	}

	/**
	 * Saves what the form holds, leaving out the fields refused. One save at a time: a change
	 * finished meanwhile is saved right after, with the latest values.
	 */
	async function save() {
		if (saving) {
			saveAgain = true;
			return;
		}
		errors = parsed.errors;
		const changes: InstanceChanges = { label: parseLabel(label), settings: parsed.settings };
		if (changes.label === (base.label ?? null) && sameJson(changes.settings, base.settings)) {
			// Back to the saved values (Escape, or the same text typed again): all is saved.
			refusal = undefined;
			saved = !invalid;
			return;
		}
		refusal = undefined;
		saved = false;
		saving = true;
		const outcome = await onapply(changes);
		saving = false;
		if (outcome.saved) {
			rebase();
			saved = true;
		} else {
			refusal = outcome.problem;
		}
		if (saveAgain) {
			saveAgain = false;
			await save();
		}
	}
</script>

<form
	class="flex flex-col gap-4"
	aria-labelledby="{uid}-title"
	onsubmit={(event) => event.preventDefault()}
	novalidate
>
	<h3 id="{uid}-title" class="font-semibold">Réglages</h3>

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

	<p role="status" class="text-sm text-muted">
		{#if invalid}
			Réglage invalide : il n’est pas enregistré tant qu’il n’est pas corrigé.
		{:else if saving}
			Enregistrement…
		{:else if saved}
			Réglages enregistrés.
		{/if}
	</p>
	<p class="text-sm text-muted">
		Les modifications sont enregistrées automatiquement. Échap dans un champ rétablit sa valeur
		enregistrée.
	</p>
</form>
