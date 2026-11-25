<!--
@component
What a page says when there is nothing to show yet: what is missing and, when there is something to do, what.
-->
<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title: string;
		/** The message appears while the page is open (a device that leaves): it is announced. */
		live?: boolean;
		children?: Snippet;
	}

	let { title, live = false, children }: Props = $props();
</script>

<section
	class="flex flex-col items-center gap-2 card px-4 py-12 text-center"
	role={live ? 'status' : undefined}
>
	<h2 class="text-lg font-semibold">{title}</h2>
	{#if children}
		<div class="flex max-w-prose flex-col items-center gap-2 text-muted">{@render children()}</div>
	{/if}
</section>
