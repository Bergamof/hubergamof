<!--
@component
Details of the selected key: position (moved with buttons, the keyboard way to reorder),
removal, key image and module settings.
-->
<script lang="ts">
	import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
	import type { InstanceChanges } from '#lib/deck/config-edits.js';
	import type { EditOutcome } from '#lib/deck/deck-state.svelte.js';
	import { instanceName } from '#lib/deck/display.js';
	import type { ManagerApi } from '#lib/api/manager-api.js';
	import InstanceEditor from './InstanceEditor.svelte';
	import KeyImagePicker from './KeyImagePicker.svelte';

	interface Props {
		position: number;
		count: number;
		instance: ModuleInstance | undefined;
		module: CatalogModule | undefined;
		serial: string;
		api: ManagerApi;
		/** Id of the heading, for the page to bring it into view. */
		headingId: string;
		onmove: (to: number) => void;
		onremove: () => void;
		onapply: (changes: InstanceChanges) => Promise<EditOutcome>;
		onimage: (image: string | null) => Promise<EditOutcome>;
	}

	let {
		position,
		count,
		instance,
		module,
		serial,
		api,
		headingId,
		onmove,
		onremove,
		onapply,
		onimage
	}: Props = $props();

	const isFirst = $derived(position === 0);
	const isLast = $derived(position === count - 1);
</script>

<section aria-labelledby={headingId} class="flex flex-col gap-4">
	<h2 id={headingId} tabindex="-1" class="text-lg font-semibold break-words">
		Touche {position + 1}{instance ? ` : ${instanceName(instance, module)}` : ''}
	</h2>
	<!-- aria-disabled rather than disabled: a disabled button would drop the keyboard focus. -->
	<div class="flex flex-wrap gap-2">
		<button
			type="button"
			class="secondary-button"
			aria-disabled={isFirst}
			onclick={() => isFirst || onmove(position - 1)}
		>
			<span aria-hidden="true">←</span> Déplacer avant
		</button>
		<button
			type="button"
			class="secondary-button"
			aria-disabled={isLast}
			onclick={() => isLast || onmove(position + 1)}
		>
			Déplacer après <span aria-hidden="true">→</span>
		</button>
		<button type="button" class="danger-button" onclick={onremove}> Supprimer la touche </button>
	</div>
	{#if instance}
		<!-- A new form for another instance, or once its module is known (catalog arriving late). -->
		{#key `${instance.id}/${module?.kind ?? ''}`}
			<KeyImagePicker {serial} {instance} {module} {api} onchange={onimage} />
			<InstanceEditor {instance} {module} {serial} {api} {onapply} />
		{/key}
	{:else}
		<p class="text-muted">Le module de cette touche est introuvable.</p>
	{/if}
</section>
