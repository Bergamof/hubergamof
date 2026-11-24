<!--
@component
A choice among a few values, shown side by side: radio buttons dressed as segments, so the keyboard (arrows)
and assistive technologies work as for any radio group. `onchange` tells what was chosen: the owner decides
when it is sent.
-->
<script lang="ts" generics="T extends string">
	interface Props {
		/** What the group is a choice of, read by assistive technologies. */
		label: string;
		options: readonly { value: T; label: string }[];
		value: T;
		onchange: (value: T) => void;
	}

	let { label, options, value, onchange }: Props = $props();

	const uid = $props.id();
</script>

<div
	role="radiogroup"
	aria-label={label}
	class="inline-flex flex-wrap gap-0.5 rounded-[10px] border border-line bg-sunken p-[3px]"
>
	{#each options as option (option.value)}
		<span class="relative">
			<input
				type="radio"
				name={uid}
				id="{uid}-{option.value}"
				value={option.value}
				class="peer absolute inset-0 size-full cursor-pointer opacity-0"
				checked={value === option.value}
				onchange={() => onchange(option.value)}
			/>
			<label
				for="{uid}-{option.value}"
				class="block cursor-pointer rounded-[7px] px-3 py-1.5 text-sm font-semibold text-muted peer-checked:bg-surface peer-checked:text-fg peer-checked:shadow-sm peer-focus-visible:outline-3 peer-focus-visible:outline-offset-1 peer-focus-visible:outline-ring"
			>
				{option.label}
			</label>
		</span>
	{/each}
</div>
