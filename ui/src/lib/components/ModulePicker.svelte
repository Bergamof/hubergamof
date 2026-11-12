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
		/** What to tell about a module besides its description, if anything. */
		notice?: (module: CatalogModule) => string | undefined;
		onpick: (module: CatalogModule) => void;
		onclose: () => void;
	}

	let { open, title, modules, notice, onpick, onclose }: Props = $props();

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
	class="m-auto w-[min(36rem,calc(100vw-2rem))] rounded-2xl border border-line bg-surface p-0 text-fg shadow-xl backdrop:bg-black/50"
	{onclose}
>
	<div class="flex items-center justify-between gap-4 border-b border-line p-4">
		<h2 id="{uid}-title" class="text-lg font-semibold">{title}</h2>
		<button type="button" class="secondary-button" onclick={onclose}> Fermer </button>
	</div>
	{#if modules.length === 0}
		<p class="p-4 text-muted">Aucun module disponible.</p>
	{:else}
		<ul class="flex max-h-[70vh] flex-col gap-1 overflow-y-auto p-2">
			{#each modules as module (module.kind)}
				<li>
					<button
						type="button"
						class="flex w-full items-center gap-3 rounded-xl p-2 text-left focus-ring hover:bg-sunken"
						onclick={() => onpick(module)}
					>
						<span class="w-14 shrink-0"><ModuleTile icon={module.icon} name={module.name} /></span>
						<span class="flex flex-col">
							<span class="font-medium">{module.name}</span>
							<span class="text-sm text-muted">{module.description}</span>
							{#if notice?.(module)}
								<span class="text-sm font-medium">{notice(module)}</span>
							{/if}
						</span>
					</button>
				</li>
			{/each}
		</ul>
	{/if}
</dialog>
