<!--
@component
A line of text with its label, an optional hint and the error found in it. The value belongs to the
caller (`oninput` gives the new one): forms keep a draft and show it here.
-->
<script lang="ts">
	interface Props {
		label: string;
		value: string;
		oninput: (value: string) => void;
		/** What the field is for, read after its label. */
		hint?: string;
		/** What is wrong with the value: shown under the field, and the field is marked invalid. */
		error?: string;
		/** Technical text (a path, a branch…) is shown in a monospaced face. */
		mono?: boolean;
		placeholder?: string;
		maxlength?: number;
		/** The kind of keyboard and checks the browser gives. */
		type?: 'text' | 'email';
		autocomplete?: 'off' | 'name' | 'email';
	}

	let {
		label,
		value,
		oninput,
		hint,
		error,
		mono = false,
		placeholder,
		maxlength,
		type = 'text',
		autocomplete = 'off'
	}: Props = $props();

	const uid = $props.id();
	let field: HTMLInputElement | undefined = $state();

	const described = $derived(
		[hint ? `${uid}-hint` : undefined, error ? `${uid}-error` : undefined].filter(Boolean).join(' ')
	);

	/** Puts the focus in the field. */
	export function focus() {
		field?.focus();
	}
</script>

<div class="flex flex-col gap-1">
	<label for="{uid}-field" class="text-sm font-medium">{label}</label>
	<input
		bind:this={field}
		id="{uid}-field"
		class={['field', mono && 'font-mono']}
		{type}
		{value}
		{placeholder}
		{maxlength}
		{autocomplete}
		autocapitalize="off"
		spellcheck="false"
		aria-invalid={error !== undefined}
		aria-describedby={described || undefined}
		oninput={(event) => oninput(event.currentTarget.value)}
	/>
	{#if hint}
		<p id="{uid}-hint" class="text-sm text-muted">{hint}</p>
	{/if}
	{#if error}
		<p id="{uid}-error" class="text-sm text-danger">{error}</p>
	{/if}
</div>
