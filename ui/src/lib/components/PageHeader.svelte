<!--
@component
Title of a page (its only `h1`), with a line of context and the actions of the page on the right.
-->
<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title: string;
		description?: string;
		/** Small status pills next to the title (connected, battery…). */
		chips?: Snippet;
		actions?: Snippet;
	}

	let { title, description, chips, actions }: Props = $props();
</script>

<div class="flex flex-wrap items-start justify-between gap-4">
	<div class="min-w-0">
		<div class="flex flex-wrap items-center gap-x-3 gap-y-1">
			<h1 class="text-2xl font-semibold">{title}</h1>
			{#if chips}{@render chips()}{/if}
		</div>
		{#if description}
			<p class="mt-0.5 text-muted">{description}</p>
		{/if}
	</div>
	{#if actions}
		<div class="flex items-center gap-2">{@render actions()}</div>
	{/if}
</div>
