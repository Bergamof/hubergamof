<!--
@component
Image of a key: a picture of the user's (PNG, JPEG or SVG, stored by the manager) or the icon
of its module. Applied right away, unlike the settings below.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
	import {
		IMAGE_TYPES,
		MAX_IMAGE_BYTES,
		type ManagerApi,
		mebibytes
	} from '#lib/api/manager-api.js';
	import type { EditOutcome } from '#lib/deck/deck-state.svelte.js';
	import { keyImage, moduleName } from '#lib/deck/display.js';
	import { Operation } from '#lib/deck/operation.svelte.js';
	import type { Problem } from '#lib/deck/problems.js';
	import ModuleTile from './ModuleTile.svelte';
	import ProblemAlert from './ProblemAlert.svelte';

	interface Props {
		serial: string;
		instance: ModuleInstance;
		module: CatalogModule | undefined;
		api: ManagerApi;
		onchange: (image: string | null) => Promise<EditOutcome>;
	}

	let { serial, instance, module, api, onchange }: Props = $props();

	const uid = $props.id();
	const operation = new Operation();
	let input: HTMLInputElement | undefined = $state();
	let chooseButton: HTMLButtonElement | undefined = $state();
	/** Image the last status is about: a change from elsewhere (import…) makes it stale. */
	let statusImage: string | null | undefined = $state();

	const status = $derived(statusImage === instance.image ? operation.status : '');

	async function saved(image: string | null): Promise<Problem | undefined> {
		const outcome = await onchange(image);
		return outcome.saved ? undefined : outcome.problem;
	}

	async function upload(file: File) {
		await operation.run(async () => {
			const uploaded = await api.uploadKeyImage(serial, file);
			if (!uploaded.ok) return uploaded.problem;
			statusImage = uploaded.value;
			return saved(uploaded.value);
		}, 'Image enregistrée.');
	}

	async function reset() {
		statusImage = null;
		const done = await operation.run(() => saved(null), 'Icône du module rétablie.');
		// The reset button is gone: keep the focus in the section.
		if (done) {
			await tick();
			chooseButton?.focus();
		}
	}

	function picked(event: Event & { currentTarget: HTMLInputElement }) {
		const file = event.currentTarget.files?.[0];
		event.currentTarget.value = '';
		if (file) void upload(file);
	}
</script>

<section aria-labelledby="{uid}-title" class="flex flex-col gap-3">
	<h3 id="{uid}-title" class="font-semibold">Image de la touche</h3>
	<div class="flex flex-wrap items-center gap-4">
		<span class="w-20 shrink-0">
			<ModuleTile
				icon={keyImage(serial, instance, module)}
				name={moduleName(instance.kind, module)}
			/>
		</span>
		<div class="flex flex-col gap-2">
			<p class="text-sm text-zinc-700 dark:text-zinc-300">
				{instance.image ? 'Image personnalisée.' : 'Icône du module.'}
			</p>
			<div class="flex flex-wrap gap-2">
				<button
					bind:this={chooseButton}
					type="button"
					class="secondary-button"
					aria-disabled={operation.busy}
					aria-describedby="{uid}-hint"
					onclick={() => operation.busy || input?.click()}
				>
					{operation.busy ? 'Envoi…' : 'Choisir une image…'}
				</button>
				{#if instance.image}
					<button
						type="button"
						class="secondary-button"
						aria-disabled={operation.busy}
						onclick={() => operation.busy || void reset()}
					>
						Rétablir l’icône du module
					</button>
				{/if}
			</div>
			<p id="{uid}-hint" class="text-sm text-zinc-600 dark:text-zinc-400">
				PNG, JPEG ou SVG, {mebibytes(MAX_IMAGE_BYTES)} au plus. Appliquée tout de suite.
			</p>
		</div>
	</div>
	<input
		bind:this={input}
		type="file"
		accept={IMAGE_TYPES.join(',')}
		class="hidden"
		tabindex="-1"
		aria-hidden="true"
		onchange={picked}
	/>
	<p role="status" class="text-sm text-zinc-700 dark:text-zinc-300">{status}</p>
	{#if operation.problem}
		<ProblemAlert problem={operation.problem} />
	{/if}
</section>
