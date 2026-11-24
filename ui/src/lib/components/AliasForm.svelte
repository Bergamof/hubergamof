<!--
@component
The form of a shell alias, to add one or to change one: its name, its command and whether it is
on the computer. The fields are checked as the manager checks them; what is wrong is said under
the field once the form was sent, and the focus goes to the first one.
-->
<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { SHELL_ALIAS_RULES, type AliasRules } from '#lib/deck/alias-rules.js';
	import { hasErrors, trimCommand } from '#lib/deck/alias-validation.js';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import type { ShellAlias } from '#lib/generated/index.js';
	import Switch from './Switch.svelte';

	interface Props {
		/** The alias being changed; none to add one. */
		initial?: ShellAlias;
		/** Names of the other aliases. */
		takenNames: readonly string[];
		submitLabel: string;
		/** Saves the alias; the form is emptied (and `ondone` called) once it was. */
		onsubmit: (alias: ShellAlias) => Promise<EditOutcome>;
		/** The change was saved. */
		ondone?: () => void;
		/** The change is given up (only when changing an alias). */
		oncancel?: () => void;
		/** Id of what names the form (a heading). */
		labelledby?: string;
		/** What the aliases must follow; the shell's by default. */
		rules?: AliasRules;
	}

	let {
		initial,
		takenNames,
		submitLabel,
		onsubmit,
		ondone,
		oncancel,
		labelledby,
		rules = SHELL_ALIAS_RULES
	}: Props = $props();

	const uid = $props.id();
	// The form starts from the alias it was opened for and keeps what is typed.
	// svelte-ignore state_referenced_locally
	let name = $state(initial?.name ?? '');
	// svelte-ignore state_referenced_locally
	let command = $state(initial?.command ?? '');
	// svelte-ignore state_referenced_locally
	let enabled = $state(initial?.enabled ?? true);
	let submitted = $state(false);
	let nameField: HTMLInputElement | undefined = $state();
	let commandField: HTMLInputElement | undefined = $state();

	const errors = $derived(
		submitted ? rules.validate({ name, command: trimCommand(command) }, takenNames) : {}
	);

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		submitted = true;
		const alias = { name, command: trimCommand(command), enabled };
		const found = rules.validate(alias, takenNames);
		if (hasErrors(found)) {
			await tick();
			(found.name ? nameField : commandField)?.focus();
			return;
		}
		const { saved } = await onsubmit(alias);
		if (!saved) return;
		name = initial?.name ?? '';
		command = initial?.command ?? '';
		enabled = initial?.enabled ?? true;
		submitted = false;
		ondone?.();
		// Adding: ready for the next alias. Changing: the owner of the form decides where the focus goes.
		if (!initial) focusName();
	}

	/** Puts the focus in the name field. */
	export function focusName() {
		nameField?.focus();
	}

	// Changing an alias: the form opens on its first field.
	onMount(() => {
		if (initial) focusName();
	});
</script>

<form
	novalidate
	aria-labelledby={labelledby}
	class="grid gap-3 sm:grid-cols-[minmax(0,14rem)_minmax(0,1fr)]"
	onsubmit={submit}
>
	<div class="flex flex-col gap-1">
		<label for="{uid}-name" class="text-sm font-medium">Nom</label>
		<input
			bind:this={nameField}
			bind:value={name}
			id="{uid}-name"
			class="field font-mono"
			autocomplete="off"
			autocapitalize="off"
			spellcheck="false"
			maxlength={rules.nameMax * 2}
			aria-invalid={errors.name !== undefined}
			aria-describedby={errors.name ? `${uid}-name-error` : undefined}
		/>
		{#if errors.name}
			<p id="{uid}-name-error" class="text-sm text-danger">{errors.name}</p>
		{/if}
	</div>
	<div class="flex flex-col gap-1">
		<label for="{uid}-command" class="text-sm font-medium">Commande</label>
		<input
			bind:this={commandField}
			bind:value={command}
			id="{uid}-command"
			class="field font-mono"
			autocomplete="off"
			autocapitalize="off"
			spellcheck="false"
			maxlength={rules.commandMax * 2}
			aria-invalid={errors.command !== undefined}
			aria-describedby={errors.command ? `${uid}-command-error` : undefined}
		/>
		{#if errors.command}
			<p id="{uid}-command-error" class="text-sm text-danger">{errors.command}</p>
		{/if}
	</div>
	<div class="flex flex-wrap items-center gap-x-4 gap-y-2 sm:col-span-2">
		<Switch checked={enabled} onchange={(checked) => (enabled = checked)}>Activé</Switch>
		<div class="flex gap-2 sm:ml-auto">
			{#if oncancel}
				<button type="button" class="secondary-button" onclick={oncancel}>Annuler</button>
			{/if}
			<button type="submit" class="primary-button">{submitLabel}</button>
		</div>
	</div>
</form>
