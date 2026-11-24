<!--
@component
The options of a host, inside the form of the host: those already written (each switched on or off,
or removed) and a pair of fields to add one. It lives in a form, which cannot hold another one, so
nothing here submits: « Ajouter l’option » is a plain button, and Enter in the fields adds the option
instead of sending the host. The options are checked as the manager checks them.
-->
<script lang="ts">
	import {
		ADVANCED_KEY_MAX,
		ADVANCED_MAX,
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
	let keyField: ReturnType<typeof TextField> | undefined = $state();
	let valueField: ReturnType<typeof TextField> | undefined = $state();

	const suffix = $derived(host === '' ? '' : ` de ${host}`);
	const full = $derived(options.length >= ADVANCED_MAX);
	const errors = $derived(
		submitted
			? validateSetting(
					{ key, value: trimCommand(value) },
					options.map((option) => option.key),
					SSH_OPTION_RULES
				)
			: {}
	);

	function add() {
		submitted = true;
		const setting = { key, value: trimCommand(value) };
		const found = validateSetting(
			setting,
			options.map((option) => option.key),
			SSH_OPTION_RULES
		);
		if (hasErrors(found)) {
			(found.name ? keyField : valueField)?.focus();
			return;
		}
		onchange([...options, { ...setting, enabled: true }]);
		key = '';
		value = '';
		submitted = false;
		keyField?.focus();
	}

	/** Enter in a field of the pair adds the option: the host is sent by its own button. */
	function addOnEnter(event: KeyboardEvent) {
		if (event.key !== 'Enter' || !(event.target instanceof HTMLInputElement)) return;
		event.preventDefault();
		add();
	}
</script>

<div
	role="group"
	aria-labelledby={labelledby}
	aria-describedby="{uid}-warning"
	class="flex flex-col gap-3"
>
	<AdvancedWarning id="{uid}-warning" tool="ssh" />
	{#if options.length > 0}
		<ul class="flex flex-col gap-2" aria-label="Options{suffix}">
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
							onchange(
								options.map((existing) =>
									existing.key === option.key ? { ...existing, enabled } : existing
								)
							)}
					/>
					<button
						type="button"
						class="secondary-button"
						aria-label="Retirer l’option {option.key}{suffix}"
						onclick={() => onchange(options.filter(({ key: other }) => other !== option.key))}
					>
						Retirer
					</button>
				</li>
			{/each}
		</ul>
	{/if}
	{#if full}
		<p class="text-sm text-muted">
			La liste contient déjà le nombre maximal d’éléments : retirez-en un pour en ajouter.
		</p>
	{:else}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div class="grid gap-3 sm:grid-cols-[minmax(0,14rem)_minmax(0,1fr)]" onkeydown={addOnEnter}>
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
				error={errors.command}
				maxlength={ADVANCED_VALUE_MAX * 2}
			/>
			<div class="sm:col-span-2">
				<button type="button" class="secondary-button" onclick={add}>Ajouter l’option</button>
			</div>
		</div>
	{/if}
</div>
