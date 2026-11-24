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
	}

	let { label, options, value, onchange, hint }: Props = $props();

	const uid = $props.id();
</script>

<div class="flex flex-col gap-1">
	<label for="{uid}-field" class="text-sm font-medium">{label}</label>
	<select
		id="{uid}-field"
		class="field"
		{value}
		aria-describedby={hint ? `${uid}-hint` : undefined}
		onchange={(event) => onchange(event.currentTarget.value as T)}
	>
		{#each options as option (option.value)}
			<option value={option.value} selected={option.value === value}>{option.label}</option>
		{/each}
	</select>
	{#if hint}
		<p id="{uid}-hint" class="text-sm text-muted">{hint}</p>
	{/if}
</div>
