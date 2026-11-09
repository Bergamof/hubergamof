<!--
@component
Label and settings form of a module instance, saved with « Appliquer ». Success is shown once
the manager has saved the change; a refusal keeps what was typed and says why. When the
instance changes elsewhere (another tab), an untouched form follows; a modified one says so
and lets the user reload it.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
	import type { InstanceChanges } from '#lib/deck/config-edits.js';
	import type { EditOutcome } from '#lib/deck/deck-state.svelte.js';
	import type { Problem } from '#lib/deck/problems.js';
	import { moduleName } from '#lib/deck/display.js';
	import { cloneJson, sameJson } from '#lib/deck/json.js';
	import {
		parseDraft,
		parseLabel,
		toDraft,
		type SettingsDraft,
		type SettingsErrors
	} from '#lib/deck/settings-form.js';
	import ProblemAlert from './ProblemAlert.svelte';
	import SettingInput from './SettingInput.svelte';

	interface Props {
		instance: ModuleInstance;
		module: CatalogModule | undefined;
		onapply: (changes: InstanceChanges) => Promise<EditOutcome>;
	}

	let { instance, module, onapply }: Props = $props();

	/** Longest label: it is drawn at the bottom of a key, where little more fits. */
	const MAX_LABEL_LENGTH = 40;

	const uid = $props.id();
	const fields = $derived(module?.settings ?? []);

	/** Plain copy of the instance as displayed now, detached from the reactive configuration. */
	function snapshot(): ModuleInstance {
		return cloneJson(instance);
	}

	/** Instance the form was filled from. */
	const initial = snapshot();
	let base = $state(initial);
	let label = $state(labelOf(initial));
	let draft: SettingsDraft = $state(draftOf(initial));
	let errors: SettingsErrors = $state({});
	let saving = $state(false);
	let saved = $state(false);
	/** « Certains réglages sont invalides. » until the next keystroke. */
	let invalid = $state(false);
	let refusal: Problem | undefined = $state();
	let form: HTMLFormElement | undefined = $state();

	const dirty = $derived(
		label !== labelOf(base) ||
			Object.entries(draftOf(base)).some(([key, value]) => draft[key] !== value)
	);
	/** While saving, the displayed instance already carries the change: not « elsewhere ». */
	const changedElsewhere = $derived(
		!saving && (base.label !== instance.label || !sameJson(base.settings, instance.settings))
	);

	// Keeps an untouched form in step with the configuration, which may change from another
	// tab at any time; a modified one is left alone (the banner below offers to reload it).
	$effect(() => {
		if (changedElsewhere && !dirty) reload();
	});

	function labelOf(source: ModuleInstance): string {
		return source.label ?? '';
	}

	function draftOf(source: ModuleInstance): SettingsDraft {
		return toDraft(fields, source.settings);
	}

	function reload() {
		base = snapshot();
		label = labelOf(base);
		draft = draftOf(base);
		errors = {};
		refusal = undefined;
		saved = false;
		invalid = false;
	}

	function edited() {
		saved = false;
		invalid = false;
	}

	async function focusFirstInvalid() {
		await tick();
		form?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus();
	}

	async function apply(event: SubmitEvent) {
		event.preventDefault();
		if (saving) return;
		const parsed = parseDraft(fields, draft, instance.settings);
		refusal = undefined;
		saved = false;
		if (!parsed.ok) {
			errors = parsed.errors;
			invalid = true;
			await focusFirstInvalid();
			return;
		}
		errors = {};
		saving = true;
		const outcome = await onapply({ label: parseLabel(label), settings: parsed.settings });
		saving = false;
		if (outcome.saved) {
			base = snapshot();
			label = labelOf(base);
			draft = draftOf(base);
			saved = true;
		} else {
			refusal = outcome.problem;
		}
	}
</script>

<form
	bind:this={form}
	class="flex flex-col gap-4"
	aria-labelledby="{uid}-title"
	onsubmit={apply}
	oninput={edited}
	novalidate
>
	<h3 id="{uid}-title" class="font-semibold">Réglages</h3>
	{#if changedElsewhere && dirty}
		<div
			role="alert"
			class="flex flex-wrap items-center gap-3 rounded-xl border border-warning bg-warning-soft p-3 text-sm text-warning"
		>
			<span>Ce module a été modifié ailleurs pendant votre saisie.</span>
			<button type="button" class="secondary-button border-current" onclick={reload}>
				Recharger
			</button>
		</div>
	{/if}

	<div class="flex flex-col gap-1">
		<label class="font-medium" for="{uid}-label">Libellé</label>
		<input
			id="{uid}-label"
			type="text"
			class="field"
			placeholder={moduleName(instance.kind, module)}
			maxlength={MAX_LABEL_LENGTH}
			bind:value={label}
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
		<SettingInput {field} idPrefix={uid} bind:value={draft[field.key]} error={errors[field.key]} />
	{/each}

	{#if refusal}
		<ProblemAlert problem={refusal} lead="Réglages non enregistrés." />
	{/if}

	<div class="flex flex-wrap items-center gap-3">
		<button
			type="submit"
			class="primary-button"
			aria-disabled={!dirty || saving}
			onclick={(event) => (dirty && !saving) || event.preventDefault()}
		>
			{saving ? 'Enregistrement…' : 'Appliquer'}
		</button>
		<button
			type="button"
			class="secondary-button border-transparent text-base"
			aria-disabled={!dirty || saving}
			onclick={() => dirty && !saving && reload()}
		>
			Annuler les modifications
		</button>
		<p role="status" class="text-sm text-muted">
			{#if invalid}
				Certains réglages sont invalides.
			{:else if saved}
				Réglages enregistrés.
			{/if}
		</p>
	</div>
</form>
