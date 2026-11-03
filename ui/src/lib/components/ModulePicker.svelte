<!--
@component
Modal list of modules to choose from (native `<dialog>`: focus trap, Escape and focus return
come with it).
-->
<script lang="ts">
	import type { CatalogModule } from '#lib/generated/index.js';
	import ModuleTile from './ModuleTile.svelte';

	interface Props {
		open: boolean;
		title: string;
		modules: readonly CatalogModule[];
		onpick: (module: CatalogModule) => void;
		onclose: () => void;
	}

	let { open, title, modules, onpick, onclose }: Props = $props();

	const uid = $props.id();
	let dialog: HTMLDialogElement | undefined = $state();

	$effect(() => {
		if (!dialog) return;
		if (open && !dialog.open) dialog.showModal();
		else if (!open && dialog.open) dialog.close();
	});
</script>

<dialog
	bind:this={dialog}
	aria-labelledby="{uid}-title"
	closedby="any"
	class="m-auto w-[min(36rem,calc(100vw-2rem))] rounded-2xl bg-white p-0 text-zinc-900 shadow-xl backdrop:bg-black/50 dark:bg-zinc-900 dark:text-zinc-100"
	{onclose}
>
	<div
		class="flex items-center justify-between gap-4 border-b border-zinc-200 p-4 dark:border-zinc-700"
	>
		<h2 id="{uid}-title" class="text-lg font-semibold">{title}</h2>
		<button
			type="button"
			class="rounded-lg px-3 py-1.5 text-sm focus-ring hover:bg-zinc-100 dark:hover:bg-zinc-800"
			onclick={onclose}
		>
			Fermer
		</button>
	</div>
	{#if modules.length === 0}
		<p class="p-4 text-zinc-600 dark:text-zinc-400">Aucun module disponible.</p>
	{:else}
		<ul class="flex max-h-[70vh] flex-col gap-1 overflow-y-auto p-2">
			{#each modules as module (module.kind)}
				<li>
					<button
						type="button"
						class="flex w-full items-center gap-3 rounded-xl p-2 text-left focus-ring hover:bg-zinc-100 dark:hover:bg-zinc-800"
						onclick={() => onpick(module)}
					>
						<span class="w-14 shrink-0"><ModuleTile icon={module.icon} name={module.name} /></span>
						<span class="flex flex-col">
							<span class="font-medium">{module.name}</span>
							<span class="text-sm text-zinc-600 dark:text-zinc-400">{module.description}</span>
						</span>
					</button>
				</li>
			{/each}
		</ul>
	{/if}
</dialog>
