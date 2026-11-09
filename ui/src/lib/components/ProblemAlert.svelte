<!--
@component
A problem shown next to what failed: French text, then the manager's details in English.
-->
<script lang="ts">
	import type { Problem } from '#lib/deck/problems.js';

	interface Props {
		problem: Problem;
		/** Sentence before the problem text (« Réglages non enregistrés. »). */
		lead?: string;
	}

	let { problem, lead }: Props = $props();
</script>

<div role="alert" class="rounded-xl border border-danger bg-danger-soft p-3 text-sm text-danger">
	<p>{lead ? `${lead} ` : ''}{problem.text}</p>
	{#if problem.detail}
		<p class="mt-1">Détail : <span lang="en">{problem.detail}</span></p>
	{/if}
</div>
