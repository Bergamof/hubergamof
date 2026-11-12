<!--
@component
Theme of the device: the colours of its keys and of its touch strip. Every change is applied at once.
-->
<script lang="ts">
	import type { DeviceTheme } from '#lib/generated/index.js';

	interface Props {
		theme: DeviceTheme;
		onchange: (theme: DeviceTheme) => void;
	}

	let { theme, onchange }: Props = $props();

	const uid = $props.id();

	const THEMES: readonly { value: DeviceTheme; label: string }[] = [
		{ value: 'light', label: 'Clair' },
		{ value: 'dark', label: 'Sombre' }
	];
</script>

<fieldset class="flex flex-wrap items-center gap-3">
	<legend class="sr-only">Thème de l’appareil</legend>
	<span class="font-medium" aria-hidden="true">Thème</span>
	{#each THEMES as choice (choice.value)}
		<label class="flex items-center gap-2">
			<input
				type="radio"
				name="{uid}-theme"
				value={choice.value}
				class="size-5 accent-ring focus-ring"
				checked={theme === choice.value}
				onchange={() => onchange(choice.value)}
			/>
			{choice.label}
		</label>
	{/each}
</fieldset>
