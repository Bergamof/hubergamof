<!--
@component
The form of a shell alias, to add one or to change one: its name, its command and whether it is
on the computer. The fields are checked as the manager checks them; what is wrong is said under
the field once the form was sent, and the focus goes to the first one.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import {
		ALIAS_COMMAND_MAX,
		ALIAS_NAME_MAX,
		hasErrors,
		validateAlias
	} from '#lib/deck/alias-validation.js';
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
	}

	let { initial, takenNames, submitLabel, onsubmit, ondone, oncancel }: Props = $props();

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
		submitted ? validateAlias({ name, command: command.trim() }, takenNames) : {}
	);

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		submitted = true;
		const alias = { name, command: command.trim(), enabled };
		const found = validateAlias(alias, takenNames);
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
		nameField?.focus();
	}
</script>

<form novalidate class="grid gap-3 sm:grid-cols-[minmax(0,14rem)_minmax(0,1fr)]" onsubmit={submit}>
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
			maxlength={ALIAS_NAME_MAX * 2}
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
			maxlength={ALIAS_COMMAND_MAX * 2}
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
