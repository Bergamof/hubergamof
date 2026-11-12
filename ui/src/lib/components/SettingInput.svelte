<!--
@component
Form control of one module setting, chosen from its kind. A choice is reported done at once
(`oncommit`); typed text is reported as it is typed (`ontype`, the form waits for a pause) and
done when the field is left or Enter is pressed in a one-line field. Escape asks for the saved
value back (`onrevert`).
-->
<script lang="ts">
	import type { SettingField } from '#lib/generated/index.js';
	import type { ManagerApi } from '#lib/api/manager-api.js';
	import { settingHint, type DraftValue } from '#lib/deck/settings-form.js';
	import FileEntryListInput from './FileEntryListInput.svelte';

	interface Props {
		field: SettingField;
		/** Unique prefix of the element ids. */
		idPrefix: string;
		value: DraftValue;
		error: string | undefined;
		/** Device and client of the files a file list stores icons for. */
		serial: string;
		api: ManagerApi;
		/** Kind of the module the setting belongs to. */
		moduleKind: string;
		/** Icon of an item without image of its own (module icon unless the module says otherwise). */
		defaultIcon: string | null | undefined;
		/** Called at each keystroke of a text: the change is not finished yet. */
		ontype: () => void;
		/** Called when a change is finished: a choice made, a text field left or confirmed. */
		oncommit: () => void;
		/** Called on Escape in a text field: the saved value is wanted back. */
		onrevert: () => void;
	}

	let {
		field,
		idPrefix,
		value = $bindable(),
		error,
		serial,
		api,
		moduleKind,
		defaultIcon,
		ontype,
		oncommit,
		onrevert
	}: Props = $props();

	/** Enter confirms a one-line text (there is no button to submit the form any more). */
	function confirmOnEnter(event: KeyboardEvent) {
		if (event.key !== 'Enter' || event.isComposing) return;
		event.preventDefault();
		oncommit();
	}

	function revertOnEscape(event: KeyboardEvent) {
		if (event.key !== 'Escape' || event.isComposing) return;
		event.preventDefault();
		onrevert();
	}

	function typed(text: string) {
		value = text;
		ontype();
	}

	const id = $derived(`${idPrefix}-${field.key}`);
	const hint = $derived(settingHint(field));
	const unit = $derived(field.kind.type === 'integer' ? field.kind.unit : null);
	const describedBy = $derived(
		[unit && `${id}-unit`, hint && `${id}-hint`, error && `${id}-error`]
			.filter(Boolean)
			.join(' ') || undefined
	);
</script>

<div class="flex flex-col gap-1">
	{#if field.kind.type === 'boolean'}
		<label class="flex items-center gap-2" for={id}>
			<input
				{id}
				type="checkbox"
				class="size-5 accent-ring focus-ring"
				checked={value === true}
				onchange={(event) => {
					value = event.currentTarget.checked;
					oncommit();
				}}
				aria-describedby={describedBy}
			/>
			{field.label}
		</label>
	{:else if field.kind.type === 'fileEntryList'}
		<span id="{id}-label" class="font-medium">{field.label}</span>
		<FileEntryListInput
			{id}
			labelledBy="{id}-label"
			{describedBy}
			kind={field.kind}
			module={moduleKind}
			setting={field.key}
			entries={Array.isArray(value) ? value : []}
			{serial}
			{api}
			{defaultIcon}
			invalid={error !== undefined}
			onchange={(entries) => {
				value = entries;
				oncommit();
			}}
		/>
	{:else}
		<label class="font-medium" for={id}>{field.label}</label>
		{#if field.kind.type === 'choice'}
			<select
				{id}
				class="field"
				value={String(value)}
				onchange={(event) => {
					value = event.currentTarget.value;
					oncommit();
				}}
				aria-describedby={describedBy}
				aria-invalid={error ? true : undefined}
			>
				{#each field.kind.options as option (option.value)}
					<option value={option.value}>{option.label}</option>
				{/each}
			</select>
		{:else if field.kind.type === 'pathList'}
			<textarea
				{id}
				rows="3"
				spellcheck="false"
				class="field"
				value={String(value)}
				oninput={(event) => typed(event.currentTarget.value)}
				onchange={oncommit}
				onkeydown={revertOnEscape}
				aria-describedby={describedBy}
				aria-invalid={error ? true : undefined}></textarea>
		{:else}
			<span class="flex items-center gap-2">
				<input
					{id}
					type="text"
					inputmode={field.kind.type === 'integer' ? 'numeric' : undefined}
					spellcheck={field.kind.type === 'text'}
					class="field"
					value={String(value)}
					oninput={(event) => typed(event.currentTarget.value)}
					onchange={oncommit}
					onkeydown={(event) => {
						confirmOnEnter(event);
						revertOnEscape(event);
					}}
					aria-describedby={describedBy}
					aria-invalid={error ? true : undefined}
				/>
				{#if unit}
					<span id="{id}-unit" class="text-sm text-muted">{unit}</span>
				{/if}
			</span>
		{/if}
	{/if}
	{#if hint}
		<p id="{id}-hint" class="text-sm text-muted">{hint}</p>
	{/if}
	{#if error}
		<p id="{id}-error" class="text-sm font-medium text-danger">{error}</p>
	{/if}
</div>
