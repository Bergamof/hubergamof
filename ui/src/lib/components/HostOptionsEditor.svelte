<!--
@component
The options of a host, inside the form of the host: those already written (each switched on or off,
or removed) and a pair of fields to add one. It lives in a form, which cannot hold another one, so
nothing here submits: « Ajouter l’option » is a plain button, and Enter in a filled field adds the
option instead of sending the host. The options are checked as the manager checks them. The form
calls `commit()` when it is sent, so that a pair typed but not added is not lost.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import {
		addSetting,
		isSettingsListFull,
		removeSetting,
		setSettingEnabled,
		SETTINGS_FULL_TEXT
	} from '#lib/deck/advanced-edits.js';
	import {
		ADVANCED_KEY_MAX,
		ADVANCED_VALUE_MAX,
		validateSetting
	} from '#lib/deck/advanced-validation.js';
	import { hasErrors, trimCommand } from '#lib/deck/alias-validation.js';
	import { SSH_OPTION_RULES } from '#lib/deck/ssh-validation.js';
	import type { AdvancedSetting } from '#lib/generated/index.js';
	import AdvancedWarning from './AdvancedWarning.svelte';
	import Switch from './Switch.svelte';
	import TextField from './TextField.svelte';

	interface Props {
		options: readonly AdvancedSetting[];
		onchange: (options: AdvancedSetting[]) => void;
		/** What the host is called, for the names of the buttons (« prod »); empty for a new host. */
		host?: string;
		/** Id of what names the group (the heading it is under). */
		labelledby: string;
	}

	let { options, onchange, host = '', labelledby }: Props = $props();

	const uid = $props.id();
	let key = $state('');
	let value = $state('');
	let submitted = $state(false);
	/** Read by screen readers when an option is removed: the button that was pressed is gone. */
	let announcement = $state('');
	let list: HTMLUListElement | undefined = $state();
	let keyField: ReturnType<typeof TextField> | undefined = $state();
	let valueField: ReturnType<typeof TextField> | undefined = $state();

	const suffix = $derived(host === '' ? '' : ` de ${host}`);
	const full = $derived(isSettingsListFull(options));
	const errors = $derived(submitted ? checked() : {});

	function checked() {
		return validateSetting(
			{ key, value: trimCommand(value) },
			options.map((option) => option.key),
			SSH_OPTION_RULES
		);
	}

	/** Adds the option of the pair, when it follows the rules; says what is wrong when it does not. */
	function add(): boolean {
		submitted = true;
		const found = checked();
		if (hasErrors(found)) {
			(found.name ? keyField : valueField)?.focus();
			return false;
		}
		onchange(
			addSetting({ key, value: trimCommand(value), enabled: true })(options as AdvancedSetting[])
		);
		key = '';
		value = '';
		submitted = false;
		keyField?.focus();
		return true;
	}

	/**
	 * Called when the host is sent: a pair left empty is nothing to add; a pair typed is added, or
	 * says what is wrong and the host is not sent.
	 */
	export function commit(): boolean {
		return key === '' && value === '' ? true : add();
	}

	/** Enter in a filled field of the pair adds the option; in an empty pair the host is sent. */
	function addOnEnter(event: KeyboardEvent) {
		if (event.key !== 'Enter' || event.isComposing || !(event.target instanceof HTMLInputElement)) {
			return;
		}
		if (key === '' && value === '') return;
		event.preventDefault();
		add();
	}

	async function remove(option: AdvancedSetting) {
		const index = options.findIndex(({ key: other }) => other === option.key);
		const neighbour = options[index + 1] ?? options[index - 1];
		onchange(removeSetting(option.key)(options as AdvancedSetting[]));
		announcement = '';
		await tick();
		announcement = `Option ${option.key} retirée${suffix}.`;
		const button = neighbour
			? list?.querySelector<HTMLButtonElement>(`[data-remove="${neighbour.key}"]`)
			: undefined;
		if (button) button.focus();
		else keyField?.focus();
	}
</script>

<div role="group" aria-labelledby={labelledby} class="flex flex-col gap-3">
	{#if options.length > 0}
		<ul bind:this={list} class="flex flex-col gap-2" aria-label="Options{suffix}">
			{#each options as option (option.key)}
				<li class="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border border-line p-3">
					<div class={['flex min-w-0 flex-1 basis-48 flex-col', !option.enabled && 'text-muted']}>
						<code class="font-mono font-semibold break-all">{option.key}</code>
						<code class="font-mono text-sm break-all">{option.value}</code>
					</div>
					<Switch
						checked={option.enabled}
						label="Option {option.key}{suffix}"
						onchange={(enabled) =>
							onchange(setSettingEnabled(option.key, enabled)(options as AdvancedSetting[]))}
					/>
					<button
						type="button"
						data-remove={option.key}
						class="secondary-button"
						aria-label="Retirer l’option {option.key}{suffix}"
						onclick={() => remove(option)}
					>
						Retirer
					</button>
				</li>
			{/each}
		</ul>
	{/if}
	<p role="status" class="sr-only">{announcement}</p>
	<AdvancedWarning id="{uid}-warning" tool="ssh" />
	{#if full}
		<p class="text-sm text-muted">{SETTINGS_FULL_TEXT}</p>
	{:else}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="grid gap-3 sm:grid-cols-[minmax(0,14rem)_minmax(0,1fr)]"
			aria-describedby="{uid}-warning"
			onkeydown={addOnEnter}
		>
			<TextField
				bind:this={keyField}
				label="Nom de l’option"
				mono
				value={key}
				oninput={(next) => (key = next)}
				placeholder="ServerAliveInterval"
				error={errors.name}
				maxlength={ADVANCED_KEY_MAX * 2}
			/>
			<TextField
				bind:this={valueField}
				label="Valeur"
				mono
				{value}
				oninput={(next) => (value = next)}
				hint="Entrée ajoute l’option."
				error={errors.command}
				maxlength={ADVANCED_VALUE_MAX * 2}
			/>
			<div class="sm:col-span-2">
				<button type="button" class="secondary-button" onclick={add}>Ajouter l’option</button>
			</div>
		</div>
	{/if}
</div>
