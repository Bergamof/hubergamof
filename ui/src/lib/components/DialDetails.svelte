<!--
@component
Details of the selected dial: which function it is given (one of the manager, or one offered by a
module, already configured or new), applied with a button so that browsing the list changes
nothing, and the settings of the module of that function.
-->
<script lang="ts">
	import type {
		CatalogModule,
		DeviceConfig,
		DialAssignment,
		ModuleInstance
	} from '#lib/generated/index.js';
	import {
		assignDial,
		removedInstance,
		type DialChoice,
		type InstanceChanges
	} from '#lib/deck/config-edits.js';
	import type { EditOutcome } from '#lib/deck/deck-state.svelte.js';
	import {
		CONTEXTUAL_OPTION,
		NAVIGATION_OPTION,
		NO_DIAL_FUNCTION,
		assignmentOption,
		choiceOf,
		existingOption,
		newOption
	} from '#lib/deck/dial-options.js';
	import { dialFunctionName, instanceName } from '#lib/deck/display.js';
	import type { ManagerApi } from '#lib/api/manager-api.js';
	import InstanceEditor from './InstanceEditor.svelte';

	interface Props {
		index: number;
		config: DeviceConfig;
		serial: string;
		api: ManagerApi;
		/** What the dial does now. */
		assignment: DialAssignment;
		/** The instance of that function, if it is one of a module. */
		instance: ModuleInstance | null;
		/** Instances offering dial functions, with where they are placed. */
		candidates: readonly { instance: ModuleInstance; place: string }[];
		/** Modules offering dial functions, for a new instance. */
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
		serial,
		api,
		assignment,
		instance,
		candidates,
		modules,
		moduleOf,
		headingId,
		onassign,
		onapply
	}: Props = $props();

	/** What the functions of the manager do, as the user reads it. */
	const HOST_DESCRIPTIONS: Readonly<Record<string, string>> = {
		[NAVIGATION_OPTION]:
			'Tourner fait défiler les lignes de la page affichée, appuyer revient à la page précédente.',
		[CONTEXTUAL_OPTION]: 'Fonction qui dépend de ce qui est affiché. Elle n’est pas encore définie.'
	};

	const uid = $props.id();
	const current = $derived(assignmentOption(assignment));
	let chosen: string | undefined = $state();
	const pending = $derived(chosen ?? current);
	const choice = $derived(choiceOf(pending, modules));
	const lost = $derived(
		pending === current ? undefined : removedInstance(config, assignDial(index, choice))
	);
	const module = $derived(instance ? moduleOf(instance.kind) : undefined);

	/** What the function being chosen does. */
	const description = $derived.by(() => {
		if (choice.type === 'existing') {
			const owner = config.modules.find((candidate) => candidate.id === choice.instance);
			const kind = owner ? moduleOf(owner.kind) : undefined;
			return kind?.dialFunctions.find((fn) => fn.id === choice.function)?.description;
		}
		if (choice.type === 'new') {
			return choice.module.dialFunctions.find((fn) => fn.id === choice.function)?.description;
		}
		return HOST_DESCRIPTIONS[pending];
	});

	function assign(event: SubmitEvent) {
		event.preventDefault();
		onassign(choice);
		chosen = undefined;
	}
</script>

<section aria-labelledby={headingId} class="flex flex-col gap-4">
	<h2 id={headingId} tabindex="-1" class="text-lg font-semibold">Molette {index + 1}</h2>
	<form class="flex flex-col gap-2" onsubmit={assign}>
		<label class="font-medium" for="{uid}-function">Fonction</label>
		<div class="flex flex-wrap gap-2">
			<select
				id="{uid}-function"
				class="field w-auto flex-1"
				value={pending}
				onchange={(event) => (chosen = event.currentTarget.value)}
				aria-describedby="{uid}-hint{lost ? ` ${uid}-lost` : ''}"
			>
				<option value={NO_DIAL_FUNCTION}>Rien</option>
				<option value={NAVIGATION_OPTION}>Navigation</option>
				<option value={CONTEXTUAL_OPTION}>Fonction contextuelle</option>
				{#if candidates.length > 0}
					<optgroup label="Fonctions des modules configurés">
						{#each candidates as candidate (candidate.instance.id)}
							{@const kind = moduleOf(candidate.instance.kind)}
							{#each kind?.dialFunctions ?? [] as fn (fn.id)}
								<option value={existingOption(candidate.instance.id, fn.id)}>
									{fn.name} ({instanceName(candidate.instance, kind)}, {candidate.place})
								</option>
							{/each}
						{/each}
					</optgroup>
				{/if}
				{#if modules.length > 0}
					<optgroup label="Nouveau module">
						{#each modules as candidate (candidate.kind)}
							{#each candidate.dialFunctions as fn (fn.id)}
								<option value={newOption(candidate, fn)}>
									{fn.name} ({candidate.name}, nouveau)
								</option>
							{/each}
						{/each}
					</optgroup>
				{/if}
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
			{#if description}{description}{/if}
			Un module déjà configuré garde ses réglages, partagés avec ses autres emplacements.
		</p>
		{#if lost}
			<p id="{uid}-lost" class="text-sm font-medium text-warning">
				« {instanceName(lost, moduleOf(lost.kind))} » n’est placé nulle part ailleurs : il sera supprimé
				avec ses réglages.
			</p>
		{/if}
	</form>
	{#if instance && assignment.kind === 'module'}
		<p class="text-sm text-muted">
			Réglages de « {instanceName(instance, module)} », qui offre « {dialFunctionName(
				module,
				assignment.function
			)} ».
		</p>
		{#key `${instance.id}/${module?.kind ?? ''}`}
			<InstanceEditor {instance} {module} {serial} {api} {onapply} />
		{/key}
	{/if}
</section>
