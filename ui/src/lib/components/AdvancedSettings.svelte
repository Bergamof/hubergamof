<!--
@component
A list of settings the person writes themselves, for what the application does not manage natively:
a key and a value, each switched on or off, changed or removed, and a form to add one. The forms and
lists of the aliases do the work, a key standing for the name and a value for the command. Every
change is handed to `onchange` as a function of the list, which the owner applies to its
configuration.
-->
<script lang="ts">
	import {
		addSetting,
		asAlias,
		fromAlias,
		isSettingsListFull,
		SETTINGS_FULL_TEXT,
		removeSetting,
		replaceSetting,
		setSettingEnabled,
		type SettingsEdit
	} from '#lib/deck/advanced-edits.js';
	import type { EntryRules } from '#lib/deck/alias-rules.js';
	import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
	import type { AdvancedSetting } from '#lib/generated/index.js';
	import AdvancedWarning from './AdvancedWarning.svelte';
	import AliasForm from './AliasForm.svelte';
	import AliasList from './AliasList.svelte';

	interface Props {
		settings: readonly AdvancedSetting[];
		/** What the keys and values must follow, and what names them. */
		rules: EntryRules;
		/** The program that reads the settings, for the warning. */
		tool: 'ssh' | 'git';
		onchange: (change: SettingsEdit) => Promise<EditOutcome>;
		/** What names the list (« Options du serveur prod »). */
		label: string;
		/** What names the form that adds one (« Ajouter une option à prod »). */
		addLabel: string;
		/** Where the settings belong, for the names of their buttons (« du serveur prod »). */
		suffix?: string;
	}

	let { settings, rules, tool, onchange, label, addLabel, suffix = '' }: Props = $props();

	const uid = $props.id();
	let addForm: ReturnType<typeof AliasForm> | undefined = $state();
	let addHeading: HTMLParagraphElement | undefined = $state();
	const aliases = $derived(settings.map(asAlias));
</script>

<div class="flex flex-col gap-3">
	<AdvancedWarning id="{uid}-warning" {tool} />
	<AliasList
		{rules}
		{label}
		{suffix}
		{aliases}
		onreplace={(key, alias) => onchange(replaceSetting(key, fromAlias(alias)))}
		ontoggle={(key, enabled) => void onchange(setSettingEnabled(key, enabled))}
		onremove={(key) => void onchange(removeSetting(key))}
		fallbackFocus={() => (addForm ? addForm.focusName() : addHeading?.focus())}
	/>
	<p
		bind:this={addHeading}
		tabindex="-1"
		id="{uid}-add"
		class="font-display text-base font-semibold"
	>
		{addLabel}
	</p>
	{#if isSettingsListFull(settings)}
		<p class="text-sm text-muted">{SETTINGS_FULL_TEXT}</p>
	{:else}
		<AliasForm
			bind:this={addForm}
			{rules}
			labelledby="{uid}-add"
			describedby="{uid}-warning"
			takenNames={settings.map(({ key }) => key)}
			submitLabel="Ajouter"
			onsubmit={(alias) => onchange(addSetting(fromAlias(alias)))}
		/>
	{/if}
</div>
