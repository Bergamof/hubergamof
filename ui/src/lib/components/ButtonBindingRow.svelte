<!--
@component
What one control of the mouse does: its own behaviour, or an action chosen here. A locked control (left
and right click) is only named. Changes are sent at once; `null` gives the control back to the mouse.
-->
<script lang="ts">
	import { ACTION_KINDS, MOUSE_BUTTONS, NATIVE, type ActionKind } from '#lib/deck/mouse-labels.js';
	import type { ButtonAction, CatalogModule } from '#lib/generated/index.js';
	import ShortcutInput from './ShortcutInput.svelte';

	interface Props {
		label: string;
		/** Cannot be given another action. */
		locked: boolean;
		action: ButtonAction | undefined;
		/** Modules, the ones offering actions can be chosen. */
		catalog: readonly CatalogModule[];
		onchange: (action: ButtonAction | null) => void;
	}

	let { label, locked, action, catalog, onchange }: Props = $props();

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

<div class="flex flex-col gap-2 py-3" role="group" aria-labelledby="{uid}-name">
	<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
		<span id="{uid}-name" class="w-56 font-medium">{label}</span>
		{#if locked}
			<span class="text-sm text-muted"
				>Garde toujours son rôle, pour que la souris reste utilisable.</span
			>
		{:else}
			<label class="sr-only" for="{uid}-kind">Action de : {label}</label>
			<select
				id="{uid}-kind"
				class="field w-auto"
				value={shownKind}
				onchange={(event) => chooseKind(event.currentTarget.value as ActionKind)}
			>
				{#each ACTION_KINDS as choice (choice.value)}
					<option value={choice.value}>{choice.label}</option>
				{/each}
			</select>
			{#if action?.kind === 'click'}
				<label class="sr-only" for="{uid}-button">Bouton cliqué par : {label}</label>
				<select
					id="{uid}-button"
					class="field w-auto"
					value={action.button}
					onchange={(event) =>
						onchange({ kind: 'click', button: event.currentTarget.value as typeof action.button })}
				>
					{#each MOUSE_BUTTONS as button (button.value)}
						<option value={button.value}>{button.label}</option>
					{/each}
				</select>
			{:else if action?.kind === 'module'}
				<label class="sr-only" for="{uid}-module">Module de : {label}</label>
				<select
					id="{uid}-module"
					class="field w-auto"
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
				<label class="sr-only" for="{uid}-module-action">Action du module de : {label}</label>
				<select
					id="{uid}-module-action"
					class="field w-auto"
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
			{/if}
		{/if}
	</div>
	{#if !locked && (action?.kind === 'shortcut' || pending)}
		<ShortcutInput
			label="Raccourci de : {label}"
			focusOnMount={pending}
			value={action?.kind === 'shortcut' ? action.keys : ''}
			onchange={(keys) => {
				pending = false;
				onchange({ kind: 'shortcut', keys });
			}}
		/>
	{/if}
</div>
