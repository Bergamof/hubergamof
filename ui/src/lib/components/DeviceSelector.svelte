<!--
@component
Choice of the device (or mouse) to configure; a single one is just named.
-->
<script lang="ts">
	/** What the choice needs of a device or a mouse. */
	interface Choice {
		serial: string;
		displayName: string;
	}

	interface Props {
		devices: readonly Choice[];
		/** Name of what is chosen. */
		label?: string;
		selected: string | undefined;
		onselect: (serial: string) => void;
	}

	let { devices, selected, onselect, label = 'Appareil' }: Props = $props();

	const uid = $props.id();
</script>

{#if devices.length === 1}
	<h2 class="text-xl font-semibold">{devices[0].displayName}</h2>
{:else}
	<div class="flex items-center gap-2">
		<label for={uid} class="font-medium">{label}</label>
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
