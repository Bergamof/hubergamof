<!--
@component
A choice among a list, as a native drop-down with its label and an optional hint. The value belongs
to the caller (`onchange` gives the new one).
-->
<script lang="ts" generics="T extends string">
	interface Props {
		label: string;
		options: readonly { value: T; label: string }[];
		value: T;
		onchange: (value: T) => void;
		/** What the choice does, read after the label. */
		hint?: string;
		/** What to beware of in the choice made, read with the hint and shown as a warning. */
		warning?: string;
	}

	let { label, options, value, onchange, hint, warning }: Props = $props();

	const uid = $props.id();

	const described = $derived(
		[hint ? `${uid}-hint` : undefined, warning ? `${uid}-warning` : undefined]
			.filter(Boolean)
			.join(' ')
	);
</script>

<div class="flex flex-col gap-1">
	<label for="{uid}-field" class="text-sm font-medium">{label}</label>
	<select
		id="{uid}-field"
		class="field"
		{value}
		aria-describedby={described || undefined}
		onchange={(event) => onchange(event.currentTarget.value as T)}
	>
		{#each options as option (option.value)}
			<option value={option.value} selected={option.value === value}>{option.label}</option>
		{/each}
	</select>
	{#if hint}
		<p id="{uid}-hint" class="text-sm text-muted">{hint}</p>
	{/if}
	{#if warning}
		<p id="{uid}-warning" class="text-sm text-danger">{warning}</p>
	{/if}
</div>
