<!--
@component
Configuration of the selected device: brightness, home keys and dials, and the details of the
selected key or dial.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import type {
		CatalogModule,
		DeviceConfig,
		DeviceInfo,
		ModuleInstance
	} from '#lib/generated/index.js';
	import {
		addKey,
		assignDial,
		canAddKey,
		canCreateInstance,
		sharedInstance,
		moveKey,
		removeKey,
		removedInstance,
		setBrightness,
		setKeyImage,
		setStrip,
		setTheme,
		updateInstance,
		type DialChoice,
		type InstanceChanges
	} from '#lib/deck/config-edits.js';
	import type { ManagerApi } from '#lib/api/manager-api.js';
	import type { DeckState, EditOutcome } from '#lib/deck/deck-state.svelte.js';
	import { dialSummary, instanceName, keyImage } from '#lib/deck/display.js';
	import BrightnessControl from './BrightnessControl.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import DialDetails from './DialDetails.svelte';
	import DialRow from './DialRow.svelte';
	import ExportImport from './ExportImport.svelte';
	import KeyDetails from './KeyDetails.svelte';
	import KeyGrid from './KeyGrid.svelte';
	import ModulePicker from './ModulePicker.svelte';
	import ThemeControl from './ThemeControl.svelte';
	import StripDetails from './StripDetails.svelte';
	import StripPreview from './StripPreview.svelte';

	interface Props {
		deck: DeckState;
		// `deck.device` and `deck.config`, passed apart once known to be defined.
		device: DeviceInfo;
		config: DeviceConfig;
		api: ManagerApi;
	}

	let { deck, device, config, api }: Props = $props();

	/** A key is identified by its position and instance: a change elsewhere drops the selection. */
	type Selection =
		| { surface: 'key'; position: number; id: string | undefined }
		| { surface: 'dial'; index: number }
		| { surface: 'strip' };

	const uid = $props.id();
	const detailsHeadingId = `${uid}-details`;
	let selection: Selection | undefined = $state();
	let pickerOpen = $state(false);
	let announcement = $state('');
	/** A removal waiting for confirmation: its instance and settings would be lost. */
	let pendingRemoval: { position: number; id: string; name: string } | undefined = $state();

	const moduleOf = (kind: string) => deck.moduleOf(kind);
	const instancesById = $derived(new Map(config.modules.map((module) => [module.id, module])));
	const keys = $derived(config.home.map((id) => instancesById.get(id)));
	const dials = $derived(
		config.dials.map((assignment) => dialSummary(assignment, instancesById, moduleOf))
	);
	const keyModules = $derived(deck.catalog.filter((module) => canAddKey(config, module)));
	const dialModules = $derived(
		deck.catalog.filter(
			(module) => module.dialFunctions.length > 0 && canCreateInstance(config, module)
		)
	);
	const dialCandidates = $derived(
		config.modules
			.filter((instance) => (moduleOf(instance.kind)?.dialFunctions.length ?? 0) > 0)
			.map((instance) => ({ instance, place: placeOf(instance.id) }))
	);

	const selected = $derived.by(() => {
		if (selection?.surface === 'key' && config.home[selection.position] === selection.id) {
			return selection;
		}
		if (selection?.surface === 'dial' && selection.index < dials.length) return selection;
		if (selection?.surface === 'strip' && dials.length > 0) return selection;
		return undefined;
	});

	/** Whether a dial uses the instance of a module that exists only once: its settings are shared. */
	function usedByDial(instance: ModuleInstance | undefined): boolean {
		return (
			instance !== undefined &&
			moduleOf(instance.kind)?.capabilities.multipleInstances === false &&
			config.dials.some(
				(assignment) => assignment.kind === 'module' && assignment.instance === instance.id
			)
		);
	}

	function placeOf(id: string): string {
		const key = config.home.indexOf(id);
		if (key >= 0) return `touche ${key + 1}`;
		const dial = config.dials.findIndex(
			(assignment) => assignment.kind === 'module' && assignment.instance === id
		);
		return `molette ${dial + 1}`;
	}

	function keyButtonId(position: number): string {
		return position < 0 ? `${uid}-add` : `${uid}-key-${position}`;
	}

	function imageOf(instance: ModuleInstance, module: CatalogModule | undefined): string | null {
		return keyImage(device.serial, instance, module);
	}

	function moduleFor(instance: ModuleInstance | null | undefined): CatalogModule | undefined {
		return instance ? moduleOf(instance.kind) : undefined;
	}

	/** Brings the details into view: below the keys on narrow screens. */
	async function revealDetails() {
		await tick();
		document.getElementById(detailsHeadingId)?.scrollIntoView({ block: 'nearest' });
	}

	function selectKey(position: number) {
		selection = { surface: 'key', position, id: config.home[position] };
		void revealDetails();
	}

	function selectDial(index: number) {
		selection = { surface: 'dial', index };
		void revealDetails();
	}

	function selectStrip() {
		selection = { surface: 'strip' };
		void revealDetails();
	}

	/** Nothing to save for a key whose instance is missing. */
	const NOTHING_TO_SAVE: Promise<EditOutcome> = Promise.resolve({ saved: true });

	function apply(instance: ModuleInstance | null | undefined, changes: InstanceChanges) {
		return instance
			? deck.edit(updateInstance(instance.id, changes), { notify: false })
			: NOTHING_TO_SAVE;
	}

	/** The settings editor of `instance` went away with refused fields: they are lost. */
	function discarded(instance: ModuleInstance | null | undefined, fields: string[]) {
		const name = instance ? instanceName(instance, moduleOf(instance.kind)) : 'ce module';
		deck.report({
			text: `Réglages de « ${name} » non enregistrés, valeur invalide : ${fields.join(', ')}.`,
			detail: null
		});
	}

	function setImage(instance: ModuleInstance | undefined, image: string | null) {
		return instance
			? deck.edit(setKeyImage(instance.id, image), { notify: false })
			: NOTHING_TO_SAVE;
	}

	async function add(module: CatalogModule) {
		const position = config.home.length;
		const shared = sharedInstance(config, module);
		void deck.edit(addKey(module));
		pickerOpen = false;
		selectKey(position);
		announcement = `Touche ${module.name} ajoutée en position ${position + 1}.${
			shared ? ' Ses réglages sont partagés avec sa molette.' : ''
		}`;
		// The picker gives the focus back to « Ajouter une touche »: the new key is the next step.
		await tick();
		document.getElementById(keyButtonId(position))?.focus();
	}

	function move(from: number, to: number) {
		void deck.edit(moveKey(from, to));
		selection = { surface: 'key', position: to, id: config.home[to] };
		announcement = `Touche déplacée en position ${to + 1} sur ${config.home.length}.`;
	}

	/** Removes the key at `position`, once confirmed if its settings would be lost. */
	function requestRemove(position: number) {
		const lost = removedInstance(config, removeKey(position));
		if (lost) {
			pendingRemoval = { position, id: lost.id, name: instanceName(lost, moduleOf(lost.kind)) };
		} else {
			void remove(position);
		}
	}

	function confirmRemoval() {
		// Another tab may have moved the keys meanwhile: only the key asked for is removed.
		if (pendingRemoval && config.home[pendingRemoval.position] === pendingRemoval.id) {
			void remove(pendingRemoval.position);
		}
		pendingRemoval = undefined;
	}

	async function remove(position: number) {
		void deck.edit(removeKey(position));
		selection = undefined;
		announcement = 'Touche supprimée.';
		await tick();
		const neighbour = Math.min(position, config.home.length - 1);
		document.getElementById(keyButtonId(neighbour))?.focus();
	}

	function assign(index: number, choice: DialChoice) {
		void deck.edit(assignDial(index, choice));
		announcement = `Fonction de la molette ${index + 1} modifiée.`;
	}
</script>

<div class="flex flex-col gap-6" data-device-theme={config.theme}>
	<p role="status" class="sr-only">{announcement}</p>

	<div class="flex flex-col gap-6 lg:flex-row lg:items-start">
		<div class="flex min-w-0 flex-col gap-6 card lg:shrink-0">
			<!-- Corps de l'appareil : touches, bande tactile (aperçu), molettes. -->
			<div
				class="flex w-fit max-w-full flex-col gap-4 rounded-3xl border border-device-line bg-device p-5"
			>
				<section aria-labelledby="{uid}-keys" class="flex flex-col gap-3">
					<h2
						id="{uid}-keys"
						class="font-mono text-xs font-medium tracking-widest text-device-muted uppercase"
					>
						Touches
					</h2>
					<KeyGrid
						{keys}
						{moduleOf}
						{imageOf}
						columns={device.capabilities.keyColumns}
						visibleRows={device.capabilities.keyRows}
						selected={selected?.surface === 'key' ? selected.position : undefined}
						buttonId={keyButtonId}
						onselect={selectKey}
						onmove={move}
						onadd={() => (pickerOpen = true)}
					/>
				</section>

				{#if dials.length > 0}
					<section aria-labelledby="{uid}-dials" class="flex flex-col gap-3">
						<h2
							id="{uid}-dials"
							class="font-mono text-xs font-medium tracking-widest text-device-muted uppercase"
						>
							Molettes
						</h2>
						<StripPreview
							strip={config.strip}
							theme={config.theme}
							{dials}
							selected={selected?.surface === 'strip'}
							onselect={selectStrip}
						/>
						<DialRow
							{dials}
							selected={selected?.surface === 'dial' ? selected.index : undefined}
							onselect={selectDial}
						/>
					</section>
				{/if}
			</div>

			<ThemeControl theme={config.theme} onchange={(theme) => deck.edit(setTheme(theme))} />
			<BrightnessControl
				brightness={config.brightness}
				onchange={(brightness) => deck.edit(setBrightness(brightness))}
			/>
			<ExportImport {device} {api} />
		</div>

		<div class="min-w-0 flex-1 scroll-mt-6 card lg:sticky lg:top-6">
			{#if selected?.surface === 'key'}
				{@const instance = keys[selected.position]}
				{@const position = selected.position}
				<KeyDetails
					{position}
					count={keys.length}
					{instance}
					module={moduleFor(instance)}
					sharedWithDial={usedByDial(instance)}
					serial={device.serial}
					{api}
					headingId={detailsHeadingId}
					onmove={(to) => move(position, to)}
					onremove={() => requestRemove(position)}
					onapply={(changes) => apply(instance, changes)}
					ondiscard={(fields) => discarded(instance, fields)}
					onimage={(image) => setImage(instance, image)}
				/>
			{:else if selected?.surface === 'strip'}
				<StripDetails
					strip={config.strip}
					headingId={detailsHeadingId}
					onchange={(changes) => void deck.edit(setStrip(changes))}
				/>
			{:else if selected?.surface === 'dial'}
				{@const index = selected.index}
				{@const assignment = config.dials[index]}
				{@const assigned = assignment.kind === 'module' ? assignment.instance : undefined}
				{@const instance = assigned ? (instancesById.get(assigned) ?? null) : null}
				{#key index}
					<DialDetails
						{index}
						{config}
						serial={device.serial}
						{api}
						{assignment}
						{instance}
						candidates={dialCandidates}
						modules={dialModules}
						{moduleOf}
						headingId={detailsHeadingId}
						onassign={(choice) => assign(index, choice)}
						onapply={(changes) => apply(instance, changes)}
						ondiscard={(fields) => discarded(instance, fields)}
					/>
				{/key}
			{:else}
				<p class="text-muted">
					Sélectionnez une touche, la bande tactile ou une molette pour la configurer.
				</p>
			{/if}
		</div>
	</div>
</div>

<ConfirmDialog
	open={pendingRemoval !== undefined}
	title="Supprimer la touche ?"
	confirmLabel="Supprimer"
	onconfirm={confirmRemoval}
	oncancel={() => (pendingRemoval = undefined)}
>
	<p>
		« {pendingRemoval?.name} » n’est placé nulle part ailleurs : ses réglages seront perdus.
	</p>
</ConfirmDialog>

<ModulePicker
	open={pickerOpen}
	title="Ajouter une touche"
	modules={keyModules}
	empty="Tous les modules qui peuvent se placer sur une touche y sont déjà : un module à instance unique n’a qu’une touche."
	notice={(module) =>
		sharedInstance(config, module)
			? 'Déjà configuré pour une molette : la touche en partagera les réglages.'
			: undefined}
	onpick={add}
	onclose={() => (pickerOpen = false)}
/>
