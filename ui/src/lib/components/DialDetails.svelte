<!--
@component
Details of the selected dial: which module drives it (an existing instance, a new one, or
none), applied with a button so that browsing the list changes nothing, and its settings.
-->
<script lang="ts">
	import type { CatalogModule, DeviceConfig, ModuleInstance } from '#lib/generated/index.js';
	import {
		assignDial,
		removedInstance,
		type DialChoice,
		type InstanceChanges
	} from '#lib/deck/config-edits.js';
	import type { EditOutcome } from '#lib/deck/deck-state.svelte.js';
	import { NO_DIAL_MODULE, choiceOf, existingOption, newOption } from '#lib/deck/dial-options.js';
	import { instanceName } from '#lib/deck/display.js';
	import InstanceEditor from './InstanceEditor.svelte';

	interface Props {
		index: number;
		config: DeviceConfig;
		instance: ModuleInstance | null;
		/** Instances that can drive a dial, with where they are placed. */
		candidates: readonly { instance: ModuleInstance; place: string }[];
		/** Modules that can drive a dial, for a new instance. */
		modules: readonly CatalogModule[];
		moduleOf: (kind: string) => CatalogModule | undefined;
		/** Id of the heading, for the page to bring it into view. */
		headingId: string;
		onassign: (choice: DialChoice) => void;
		onapply: (changes: InstanceChanges) => Promise<EditOutcome>;
	}

	let {
		index,
		config,
		instance,
		candidates,
		modules,
		moduleOf,
		headingId,
		onassign,
		onapply
	}: Props = $props();

	const uid = $props.id();
	const current = $derived(instance ? existingOption(instance.id) : NO_DIAL_MODULE);
	let chosen: string | undefined = $state();
	const pending = $derived(chosen ?? current);
	const choice = $derived(choiceOf(pending, modules));
	const lost = $derived(
		pending === current ? undefined : removedInstance(config, assignDial(index, choice))
	);
	const module = $derived(instance ? moduleOf(instance.kind) : undefined);

	function assign(event: SubmitEvent) {
		event.preventDefault();
		onassign(choice);
		chosen = undefined;
	}
</script>

<section aria-labelledby={headingId} class="flex flex-col gap-4">
	<h2 id={headingId} tabindex="-1" class="text-lg font-semibold">Molette {index + 1}</h2>
	<form class="flex flex-col gap-2" onsubmit={assign}>
		<label class="font-medium" for="{uid}-module">Module</label>
		<div class="flex flex-wrap gap-2">
			<select
				id="{uid}-module"
				class="field w-auto flex-1"
				value={pending}
				onchange={(event) => (chosen = event.currentTarget.value)}
				aria-describedby="{uid}-hint{lost ? ` ${uid}-lost` : ''}"
			>
				<option value={NO_DIAL_MODULE}>Aucun</option>
				{#if candidates.length > 0}
					<optgroup label="Modules déjà configurés">
						{#each candidates as candidate (candidate.instance.id)}
							<option value={existingOption(candidate.instance.id)}>
								{instanceName(candidate.instance, moduleOf(candidate.instance.kind))} ({candidate.place})
							</option>
						{/each}
					</optgroup>
				{/if}
				<optgroup label="Nouveau module">
					{#each modules as candidate (candidate.kind)}
						<option value={newOption(candidate)}>{candidate.name} (nouveau)</option>
					{/each}
				</optgroup>
			</select>
			<button
				type="submit"
				class="secondary-button"
				aria-disabled={pending === current}
				onclick={(event) => pending === current && event.preventDefault()}
			>
				Affecter
			</button>
		</div>
		<p id="{uid}-hint" class="text-sm text-muted">
			Un module déjà configuré garde ses réglages, partagés avec ses autres emplacements.
		</p>
		{#if lost}
			<p id="{uid}-lost" class="text-sm font-medium text-warning">
				« {instanceName(lost, moduleOf(lost.kind))} » n’est placé nulle part ailleurs : il sera supprimé
				avec ses réglages.
			</p>
		{/if}
	</form>
	{#if instance}
		{#key `${instance.id}/${module?.kind ?? ''}`}
			<InstanceEditor {instance} {module} {onapply} />
		{/key}
	{/if}
</section>
