<!--
@component
Modal confirmation of an action that cannot be undone (native `<dialog>`: focus trap, Escape
cancels and focus returns to the opener). « Annuler », first, has the initial focus.
-->
<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		open: boolean;
		title: string;
		confirmLabel: string;
		children: Snippet;
		onconfirm: () => void | Promise<void>;
		oncancel: () => void;
	}

	let { open, title, confirmLabel, children, onconfirm, oncancel }: Props = $props();

	const uid = $props.id();
	let dialog: HTMLDialogElement | undefined = $state();
	/** Set by « Remplacer »: the `close` event that follows is not a cancellation. */
	let confirmed = false;

	function confirm() {
		confirmed = true;
		void onconfirm();
	}

	function closed() {
		if (!confirmed) oncancel();
		confirmed = false;
	}

	$effect(() => {
		if (!dialog) return;
		if (open && !dialog.open) dialog.showModal();
		else if (!open && dialog.open) dialog.close();
	});
</script>

<dialog
	bind:this={dialog}
	aria-labelledby="{uid}-title"
	aria-describedby="{uid}-message"
	class="m-auto w-[min(32rem,calc(100vw-2rem))] rounded-2xl bg-white p-6 text-zinc-900 shadow-xl backdrop:bg-black/50 dark:bg-zinc-900 dark:text-zinc-100"
	onclose={closed}
>
	<h2 id="{uid}-title" class="text-lg font-semibold">{title}</h2>
	<div id="{uid}-message" class="mt-3 text-sm">{@render children()}</div>
	<div class="mt-6 flex flex-wrap justify-end gap-3">
		<!-- First control: `showModal` focuses it, the safe choice of a confirmation. -->
		<button type="button" class="secondary-button" onclick={oncancel}>Annuler</button>
		<button
			type="button"
			class="rounded-lg bg-red-700 px-4 py-1.5 text-sm font-medium text-white focus-ring hover:bg-red-800"
			onclick={confirm}
		>
			{confirmLabel}
		</button>
	</div>
</dialog>
