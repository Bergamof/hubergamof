<!--
@component
The modules that live in the application, whether or not a device is connected: what each shows
now (the pomodoro: the step and the time left), the actions it offers, and its settings. What it
shows follows the manager at once, the keys of the decks and the radial menu showing the same.
Settings are saved as they are edited, like those of a key.
-->
<script lang="ts">
	import type { ManagerApi } from '#lib/api/manager-api.js';
	import type { DeckState } from '#lib/deck/deck-state.svelte.js';
	import { moduleName } from '#lib/deck/display.js';
	import type { ApplicationModuleView, ModuleInstance } from '#lib/generated/index.js';
	import InstanceEditor from './InstanceEditor.svelte';

	interface Props {
		deck: DeckState;
		api: ManagerApi;
	}

	let { deck, api }: Props = $props();

	const uid = $props.id();

	/** The module as an instance, which is what the settings form edits. */
	function asInstance(view: ApplicationModuleView): ModuleInstance {
		return {
			id: `application-${view.kind}`,
			kind: view.kind,
			label: null,
			image: null,
			settings: view.settings
		};
	}

	function discarded(name: string, fields: string[]) {
		deck.report({
			text: `Réglages de « ${name} » non enregistrés, valeur invalide : ${fields.join(', ')}.`,
			detail: null
		});
	}
</script>

<div class="flex flex-col gap-8">
	{#each deck.application.modules as view (view.kind)}
		{@const module = deck.moduleOf(view.kind)}
		{@const name = moduleName(view.kind, module)}
		<section aria-labelledby="{uid}-{view.kind}" class="flex flex-col gap-4 card">
			<h2 id="{uid}-{view.kind}" class="font-display text-xl font-semibold">{name}</h2>
			{#if module?.description}
				<p class="text-sm text-muted">{module.description}</p>
			{/if}
			<p class="text-sm text-muted">
				Ce module tourne dans le gestionnaire : il continue sans Stream Deck, et la touche, la
				molette, la bande tactile et le menu circulaire montrent le même.
			</p>

			{#if view.status}
				<!-- Changes every second: not announced. -->
				<div
					aria-live="off"
					aria-label="État de {name}"
					role="group"
					data-testid="module-status-{view.kind}"
					class="flex flex-col font-display"
				>
					{#each view.status.split('\n') as line, index (index)}
						<span class={index === 0 ? 'text-lg font-semibold' : 'text-3xl font-semibold'}>
							{line}
						</span>
					{/each}
				</div>
			{/if}

			{#if module && module.radialActions.length > 0}
				<ul class="flex flex-wrap gap-3" aria-label="Actions de {name}">
					{#each module.radialActions as action, index (action.id)}
						<li class="flex flex-col gap-1">
							<button
								type="button"
								class={['focus-ring', index === 0 ? 'primary-button' : 'secondary-button']}
								aria-describedby="{uid}-{view.kind}-{action.id}"
								onclick={() => deck.application.run(view.kind, action.id)}
							>
								{action.name}
							</button>
							<span id="{uid}-{view.kind}-{action.id}" class="max-w-56 text-xs text-muted">
								{action.description}
							</span>
						</li>
					{/each}
				</ul>
			{/if}

			{#if module && module.settings.length > 0}
				<div class="flex flex-col gap-2">
					<h3 class="font-semibold">Réglages</h3>
					<InstanceEditor
						instance={asInstance(view)}
						{module}
						serial=""
						{api}
						showLabel={false}
						onapply={(changes) => deck.application.saveSettings(view.kind, changes.settings)}
						ondiscard={(fields) => discarded(name, fields)}
					/>
				</div>
			{/if}
		</section>
	{/each}
</div>
