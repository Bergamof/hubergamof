<!--
@component
Drawing of a dial: a knurled ring around a cap with a notch. The notch takes the accent when a
module is assigned and fades when the dial is free. Decorative: the surrounding control carries
the accessible name.
-->
<script lang="ts">
	interface Props {
		/** A module is assigned to the dial. */
		assigned: boolean;
	}

	let { assigned }: Props = $props();
</script>

<svg viewBox="0 0 64 64" aria-hidden="true" class="aspect-square w-full" data-assigned={assigned}>
	<circle
		cx="32"
		cy="32"
		r="30"
		stroke-width="2"
		class={['fill-device-key stroke-device-line', !assigned && '[stroke-dasharray:4_4]']}
	/>
	<circle cx="32" cy="32" r="22" stroke-width="1.5" class="fill-device-strip stroke-device-line" />
	<path
		d="M32 13v10"
		stroke-width="4"
		stroke-linecap="round"
		class={assigned ? 'stroke-accent' : 'stroke-device-line'}
	/>
</svg>
