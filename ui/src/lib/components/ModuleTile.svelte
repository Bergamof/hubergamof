<!--
@component
Square standing for a module, drawn like a device key: its icon on the screen of the
device (the background of its theme, the icons being transparent), or its name when it has none. Decorative: the
surrounding control carries the accessible name.
-->
<script lang="ts">
	interface Props {
		/** Icon data URL, if the module has one. */
		icon: string | null | undefined;
		/** Fallback text when there is no icon. */
		name: string;
		/** Drawn apart, for a module this manager does not provide. */
		unavailable?: boolean;
	}

	let { icon, name, unavailable = false }: Props = $props();
</script>

<span
	aria-hidden="true"
	class={[
		'flex aspect-square w-full items-center justify-center overflow-hidden rounded-xl bg-screen text-center text-xs font-medium text-screen-ink',
		unavailable ? 'border-2 border-dashed border-device-muted' : 'ring-1 ring-device-line'
	]}
>
	{#if icon}
		<img src={icon} alt="" class="size-full object-contain" draggable="false" />
	{:else}
		<span class="line-clamp-3 px-1 break-words">{name}</span>
	{/if}
</span>
