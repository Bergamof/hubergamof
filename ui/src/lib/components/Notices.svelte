<!--
@component
Errors and warnings (from the manager or the editor), kept until dismissed. Dismissing one moves the focus to the
next message, or to a fallback element, rather than losing it.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import type { Notice } from '#lib/deck/deck-state.svelte.js';

	interface Props {
		notices: readonly Notice[];
		/** Element focused once the last message is dismissed (the list is then hidden). */
		fallbackFocusId: string;
		ondismiss: (id: number) => void;
	}

	let { notices, fallbackFocusId, ondismiss }: Props = $props();

	let list: HTMLElement | undefined = $state();

	async function dismiss(id: number) {
		ondismiss(id);
		await tick();
		const next = list?.querySelector<HTMLElement>('button');
		(next ?? document.getElementById(fallbackFocusId))?.focus();
	}
</script>

<div bind:this={list} role="alert" class="flex flex-col gap-2 empty:hidden">
	{#each notices as notice (notice.id)}
		<div
			class="flex items-start justify-between gap-4 rounded-xl border border-danger bg-danger-soft p-3 text-sm text-danger"
		>
			<div>
				<p>{notice.text}</p>
				{#if notice.detail}
					<p class="mt-1">Détail : <span lang="en">{notice.detail}</span></p>
				{/if}
			</div>
			<button
				type="button"
				class="min-h-6 shrink-0 rounded px-2 py-0.5 font-medium underline focus-ring"
				onclick={() => dismiss(notice.id)}
			>
				Fermer le message
			</button>
		</div>
	{/each}
</div>
