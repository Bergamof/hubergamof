<!--
@component
What one control of the mouse does: its own behaviour, or an action chosen here, with the module and its
action or the shortcut that goes with it. Changes are sent at once; `null` gives the control back to the
mouse.
-->
<script lang="ts">
	import { ACTION_KINDS, MOUSE_BUTTONS, NATIVE, type ActionKind } from '#lib/deck/mouse-labels.js';
	import type { ButtonAction, CatalogModule } from '#lib/generated/index.js';
	import ShortcutInput from './ShortcutInput.svelte';

	interface Props {
		action: ButtonAction | undefined;
		/** Modules, the ones offering actions can be chosen. */
		catalog: readonly CatalogModule[];
		onchange: (action: ButtonAction | null) => void;
	}

	let { action, catalog, onchange }: Props = $props();

	const uid = $props.id();

	/** The shortcut kind chosen, its keys not given yet. */
	let pending = $state(false);
	const kind: ActionKind = $derived(action?.kind ?? NATIVE);
	const shownKind: ActionKind = $derived(pending ? 'shortcut' : kind);
	const modules = $derived(catalog.filter((module) => module.radialActions.length > 0));
	const chosenModule = $derived(
		action?.kind === 'module' ? modules.find((module) => module.kind === action.module) : undefined
	);

	/** The action a kind starts with, `null` for the mouse's own behaviour. */
	function initial(next: ActionKind): ButtonAction | null {
		switch (next) {
			case NATIVE:
				return null;
			case 'click':
				return { kind: 'click', button: 'middle' };
			case 'module': {
				const first = modules[0];
				return first
					? { kind: 'module', module: first.kind, action: first.radialActions[0].id }
					: null;
			}
			case 'shortcut':
				return { kind: 'shortcut', keys: '' };
			default:
				return { kind: next } as ButtonAction;
		}
	}

	function chooseKind(next: ActionKind) {
		// A shortcut without keys is not an action: wait for the keys.
		if (next === 'shortcut') return void (pending = true);
		pending = false;
		onchange(initial(next));
	}
</script>

<div class="flex flex-col gap-3">
	<div class="flex flex-col gap-1">
		<label class="font-semibold" for="{uid}-kind">Action</label>
		<select
			id="{uid}-kind"
			class="field"
			value={shownKind}
			onchange={(event) => chooseKind(event.currentTarget.value as ActionKind)}
		>
			{#each ACTION_KINDS as choice (choice.value)}
				<option value={choice.value}>{choice.label}</option>
			{/each}
		</select>
	</div>

	{#if action?.kind === 'click'}
		<div class="flex flex-col gap-1">
			<label class="font-semibold" for="{uid}-button">Bouton cliqué</label>
			<select
				id="{uid}-button"
				class="field"
				value={action.button}
				onchange={(event) =>
					onchange({ kind: 'click', button: event.currentTarget.value as typeof action.button })}
			>
				{#each MOUSE_BUTTONS as button (button.value)}
					<option value={button.value}>{button.label}</option>
				{/each}
			</select>
		</div>
	{:else if action?.kind === 'module'}
		<div class="grid gap-3 sm:grid-cols-2">
			<div class="flex flex-col gap-1">
				<label class="font-semibold" for="{uid}-module">Module</label>
				<select
					id="{uid}-module"
					class="field"
					value={action.module}
					onchange={(event) => {
						const module = modules.find((entry) => entry.kind === event.currentTarget.value);
						if (module) {
							onchange({ kind: 'module', module: module.kind, action: module.radialActions[0].id });
						}
					}}
				>
					{#each modules as module (module.kind)}
						<option value={module.kind}>{module.name}</option>
					{/each}
					{#if !chosenModule}
						<option value={action.module}>{action.module} (absent)</option>
					{/if}
				</select>
			</div>
			<div class="flex flex-col gap-1">
				<label class="font-semibold" for="{uid}-module-action">Action du module</label>
				<select
					id="{uid}-module-action"
					class="field"
					value={action.action}
					onchange={(event) =>
						onchange({ kind: 'module', module: action.module, action: event.currentTarget.value })}
				>
					{#each chosenModule?.radialActions ?? [] as radial (radial.id)}
						<option value={radial.id}>{radial.name}</option>
					{/each}
					{#if chosenModule && !chosenModule.radialActions.some((radial) => radial.id === action.action)}
						<option value={action.action}>{action.action} (absente)</option>
					{/if}
				</select>
			</div>
		</div>
	{/if}

	{#if action?.kind === 'shortcut' || pending}
		<ShortcutInput
			label="Raccourci"
			focusOnMount={pending}
			value={action?.kind === 'shortcut' ? action.keys : ''}
			onchange={(keys) => {
				pending = false;
				onchange({ kind: 'shortcut', keys });
			}}
		/>
	{/if}

	{#if action !== undefined || pending}
		<div>
			<button
				type="button"
				class="secondary-button"
				onclick={() => {
					pending = false;
					onchange(null);
				}}
			>
				Rétablir l’action d’origine
			</button>
		</div>
	{/if}
</div>
