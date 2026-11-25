<!--
@component
One setting of a mouse: its name and what it does on one side, its control on the other. Rows stack with a
line between them.
-->
<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		label: string;
		/** What the setting does, in a sentence. */
		description: string;
		/** The control. */
		children: Snippet;
	}

	let { label, description, children }: Props = $props();
</script>

<div
	class="grid grid-cols-1 items-center gap-x-6 gap-y-1 border-b border-line py-2.5 last:border-b-0 sm:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)]"
>
	<div>
		<div class="font-medium">{label}</div>
		<div class="text-xs text-muted">{description}</div>
	</div>
	<div class="flex min-w-0 flex-wrap items-center gap-2.5">
		{@render children()}
	</div>
</div>
