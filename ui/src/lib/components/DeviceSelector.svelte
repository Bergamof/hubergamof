<!--
@component
Choice of the device to configure; a single device is just named.
-->
<script lang="ts">
	import type { DeviceInfo } from '#lib/generated/index.js';

	interface Props {
		devices: readonly DeviceInfo[];
		selected: string | undefined;
		onselect: (serial: string) => void;
	}

	let { devices, selected, onselect }: Props = $props();

	const uid = $props.id();
</script>

{#if devices.length === 1}
	<h2 class="text-xl font-semibold">{devices[0].displayName}</h2>
{:else}
	<div class="flex items-center gap-2">
		<label for={uid} class="font-medium">Appareil</label>
		<select
			id={uid}
			class="field w-auto"
			value={selected}
			onchange={(event) => onselect(event.currentTarget.value)}
		>
			{#each devices as device (device.serial)}
				<option value={device.serial}>{device.displayName}</option>
			{/each}
		</select>
	</div>
{/if}
