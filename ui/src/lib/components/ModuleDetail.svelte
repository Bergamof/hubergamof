<!--
@component
One module of the application: what it says now (the pomodoro: the step and the time left), the actions it offers
and its settings. It runs in the manager, with or without a device: the keys of the decks and the radial menu show
the same. Settings are saved as they are edited, like those of a key.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import type { ManagerApi } from '#lib/api/manager-api.js';
	import type { DeckState } from '#lib/deck/deck-state.svelte.js';
	import { moduleName } from '#lib/deck/display.js';
	import {
		availableActions,
		groupActions,
		isEnding,
		noActionText
	} from '#lib/deck/module-actions.js';
	import type { ApplicationModuleView, ModuleInstance } from '#lib/generated/index.js';
	import InstanceEditor from './InstanceEditor.svelte';

	interface Props {
		deck: DeckState;
		api: ManagerApi;
		view: ApplicationModuleView;
	}

	let { deck, api, view }: Props = $props();

	const uid = $props.id();
	const module = $derived(deck.moduleOf(view.kind));
	const name = $derived(moduleName(view.kind, module));
	/** The actions that make sense now (the call controls of Teams only exist during a call), grouped. */
	const actions = $derived(availableActions(module, view));
	const groups = $derived(groupActions(view.kind, actions));

	let section: HTMLElement | undefined = $state();
	let heading: HTMLHeadingElement | undefined = $state();

	// An action that goes away (hang up, then the call is over) takes the focus with it: the title of
	// the module gets it, rather than the body of the page.
	$effect.pre(() => {
		void actions;
		const focused = document.activeElement;
		if (!(focused instanceof HTMLElement) || !section?.contains(focused)) return;
		void tick().then(() => {
			if (!focused.isConnected) heading?.focus();
		});
	});

	/** The module as an instance, which is what the settings form edits. */
	const instance = $derived<ModuleInstance>({
		id: `application-${view.kind}`,
		kind: view.kind,
		label: null,
		image: null,
		settings: view.settings
	});

	/** Runs an action; the connection being lost, nothing was sent, which is said. */
	function run(action: string) {
		if (!deck.application.run(view.kind, action)) {
			deck.report({
				text: 'Action non envoyée : la connexion au gestionnaire est perdue.',
				detail: null
			});
		}
	}

	/** The step, first line of what the module says: it changes rarely, unlike the time. */
	const step = $derived(view.status?.split('\n')[0] ?? '');

	function discarded(fields: string[]) {
		deck.report({
			text: `Réglages de « ${name} » non enregistrés, valeur invalide : ${fields.join(', ')}.`,
			detail: null
		});
	}
</script>

<section bind:this={section} aria-labelledby="{uid}-title" class="flex flex-col gap-4 card">
	<div class="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
		<div class="min-w-0">
			<h2
				bind:this={heading}
				id="{uid}-title"
				tabindex="-1"
				class="font-display text-xl font-semibold"
			>
				{name}
			</h2>
			{#if module?.description}
				<p class="text-muted">{module.description}</p>
			{/if}
		</div>
		{#if view.status}
			<!-- The time changes every second and is not announced; the step is, below. -->
			<div
				aria-label="État de {name}"
				role="group"
				data-testid="module-status-{view.kind}"
				class="flex flex-col items-end font-display"
			>
				{#each view.status.split('\n') as line, index (index)}
					<span class={index === 0 ? 'font-semibold' : 'text-2xl font-semibold tabular-nums'}>
						{line}
					</span>
				{/each}
			</div>
		{/if}
	</div>

	<p role="status" class="sr-only">{step}</p>

	{#if module && module.radialActions.length > 0}
		<div class="flex flex-col gap-3" role="group" aria-label="Actions de {name}">
			{#each groups as group, groupIndex (group.label ?? '')}
				<div class="flex flex-wrap items-center gap-x-3 gap-y-2">
					{#if group.label}
						<h3 class="w-full text-sm font-semibold text-muted">{group.label}</h3>
					{/if}
					<ul class="flex flex-wrap gap-2">
						{#each group.actions as action, index (action.id)}
							<li>
								<button
									type="button"
									class={[
										'focus-ring',
										isEnding(action)
											? 'danger-button'
											: groupIndex === 0 && index === 0
												? 'primary-button'
												: 'secondary-button'
									]}
									title={action.description}
									aria-describedby="{uid}-{action.id}"
									onclick={() => run(action.id)}
								>
									{action.name}
								</button>
								<span id="{uid}-{action.id}" class="sr-only">{action.description}</span>
							</li>
						{/each}
					</ul>
				</div>
			{/each}
			<!-- Always there, so that what it says when the actions go away is announced. -->
			<p role="status" class="text-sm text-muted empty:hidden">
				{groups.length === 0 ? noActionText(view.kind) : ''}
			</p>
		</div>
	{/if}

	{#if module && module.settings.length > 0}
		<InstanceEditor
			{instance}
			{module}
			serial=""
			{api}
			showLabel={false}
			onapply={(changes) => deck.application.saveSettings(view.kind, changes.settings)}
			ondiscard={discarded}
		/>
	{/if}
</section>
