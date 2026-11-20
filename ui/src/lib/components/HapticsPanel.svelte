<!--
@component
Vibrations of a haptic mouse: the strength (which also silences everything when off) and, for each
event a module can signal, whether it is played and how. An event without a choice of its own plays
the way its module made it. Every change is applied at once.
-->
<script lang="ts">
	import { hapticKey } from '#lib/deck/mouse-edits.js';
	import { HAPTIC_LEVELS, HAPTIC_WAVEFORMS } from '#lib/deck/mouse-labels.js';
	import type {
		CatalogModule,
		HapticEventDescriptor,
		HapticEventSetting,
		HapticLevel,
		HapticSettings,
		HapticWaveform
	} from '#lib/generated/index.js';

	interface Props {
		haptics: HapticSettings;
		catalog: readonly CatalogModule[];
		onlevel: (level: HapticLevel) => void;
		/** `null`: back to what the module chose. */
		onevent: (key: string, setting: HapticEventSetting | null) => void;
	}

	let { haptics, catalog, onlevel, onevent }: Props = $props();

	const uid = $props.id();

	const modules = $derived(catalog.filter((module) => module.hapticEvents.length > 0));

	/** How the event is played now: the choice made here, else the module's default. */
	function settingOf(module: CatalogModule, event: HapticEventDescriptor): HapticEventSetting {
		return (
			haptics.events[hapticKey(module.kind, event.id)] ?? {
				enabled: true,
				waveform: event.waveform
			}
		);
	}

	function change(
		module: CatalogModule,
		event: HapticEventDescriptor,
		change: Partial<HapticEventSetting>
	) {
		const next = { ...settingOf(module, event), ...change };
		const isDefault = next.enabled && next.waveform === event.waveform;
		onevent(hapticKey(module.kind, event.id), isDefault ? null : next);
	}
</script>

<div class="flex flex-col gap-5">
	<div class="flex flex-col gap-1">
		<label class="font-medium" for="{uid}-level">Force des vibrations</label>
		<select
			id="{uid}-level"
			class="field w-auto self-start"
			value={haptics.level}
			onchange={(event) => onlevel(event.currentTarget.value as HapticLevel)}
		>
			{#each HAPTIC_LEVELS as level (level.value)}
				<option value={level.value}>{level.label}</option>
			{/each}
		</select>
		<p class="text-sm text-muted">
			« Aucune vibration » coupe tout sur cette souris, y compris le menu circulaire.
		</p>
	</div>

	{#each modules as module (module.kind)}
		<fieldset class="flex flex-col gap-3">
			<legend class="font-medium">{module.name}</legend>
			{#each module.hapticEvents as event (event.id)}
				{@const setting = settingOf(module, event)}
				<div class="flex flex-col gap-1">
					<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
						<label class="flex w-64 items-center gap-2">
							<input
								type="checkbox"
								class="size-5 accent-ring focus-ring"
								checked={setting.enabled}
								onchange={(e) => change(module, event, { enabled: e.currentTarget.checked })}
							/>
							{event.name}
						</label>
						<label class="sr-only" for="{uid}-{module.kind}-{event.id}">
							Vibration de : {module.name}, {event.name}
						</label>
						<select
							id="{uid}-{module.kind}-{event.id}"
							class="field w-auto"
							value={setting.waveform}
							disabled={!setting.enabled}
							onchange={(e) =>
								change(module, event, { waveform: e.currentTarget.value as HapticWaveform })}
						>
							{#each HAPTIC_WAVEFORMS as waveform (waveform.value)}
								<option value={waveform.value}>{waveform.label}</option>
							{/each}
						</select>
					</div>
					<p class="text-sm text-muted">{event.description}</p>
				</div>
			{/each}
		</fieldset>
	{:else}
		<p class="text-sm text-muted">Aucun module ne signale d’événement par vibration.</p>
	{/each}
</div>
