<!--
@component
Pointer and wheels of a mouse: sensitivity, wheel mode and direction, and the thumb wheel when the model
has one. Every change is applied at once.
-->
<script lang="ts">
	import {
		DPI_MAX,
		DPI_MIN,
		DPI_STEP,
		THUMB_SPEED_MAX,
		THUMB_SPEED_MIN,
		WHEEL_THRESHOLD_MAX,
		WHEEL_THRESHOLD_MIN
	} from '#lib/deck/mouse-edits.js';
	import { throttle } from '#lib/deck/throttle.js';
	import { WHEEL_MODES } from '#lib/deck/mouse-labels.js';
	import type {
		PointerSettings,
		ThumbWheelSettings,
		WheelMode,
		WheelSettings
	} from '#lib/generated/index.js';
	import IntegerField from './IntegerField.svelte';
	import SegmentedControl from './SegmentedControl.svelte';
	import Switch from './Switch.svelte';

	interface Props {
		pointer: PointerSettings;
		wheel: WheelSettings;
		thumbWheel: ThumbWheelSettings;
		/** The model has a thumb wheel. */
		hasThumbWheel: boolean;
		onpointer: (dpi: number) => void;
		onwheel: (change: Partial<WheelSettings>) => void;
		onthumbwheel: (change: Partial<ThumbWheelSettings>) => void;
	}

	let { pointer, wheel, thumbWheel, hasThumbWheel, onpointer, onwheel, onthumbwheel }: Props =
		$props();

	const uid = $props.id();

	/** Longest delay between two updates sent while dragging. */
	const SEND_PERIOD_MS = 150;
	const send = throttle((dpi: number) => onpointer(dpi), SEND_PERIOD_MS);

	$effect(() => () => send.cancel());
</script>

<div class="flex flex-col gap-5">
	<div class="flex flex-col gap-1">
		<label class="font-medium" for="{uid}-dpi">Sensibilité du pointeur</label>
		<span class="flex flex-wrap items-center gap-3">
			<input
				id="{uid}-dpi"
				type="range"
				min={DPI_MIN}
				max={DPI_MAX}
				step={DPI_STEP}
				value={pointer.dpi}
				aria-valuetext="{pointer.dpi} dpi"
				aria-describedby="{uid}-dpi-hint"
				class="h-6 w-64 accent-ring focus-ring"
				oninput={(event) => send(event.currentTarget.valueAsNumber)}
				onchange={(event) => send.flush(event.currentTarget.valueAsNumber)}
			/>
			<output for="{uid}-dpi" aria-hidden="true" class="w-20 tabular-nums">{pointer.dpi} dpi</output
			>
		</span>
		<p id="{uid}-dpi-hint" class="text-sm text-muted">
			De {DPI_MIN} à {DPI_MAX}, par pas de {DPI_STEP}.
		</p>
	</div>

	<div class="flex flex-col gap-1">
		<span class="font-medium" id="{uid}-mode">Molette</span>
		<SegmentedControl
			label="Molette"
			options={WHEEL_MODES}
			value={wheel.mode}
			onchange={(mode) => onwheel({ mode })}
		/>
	</div>

	{#if wheel.mode === 'auto'}
		<IntegerField
			label="Seuil de vitesse du mode automatique"
			value={wheel.autoThreshold}
			min={WHEEL_THRESHOLD_MIN}
			max={WHEEL_THRESHOLD_MAX}
			unit=""
			hint="Vitesse à partir de laquelle la molette quitte ses crans : de {WHEEL_THRESHOLD_MIN} à {WHEEL_THRESHOLD_MAX}."
			error="Entrez un nombre entier de {WHEEL_THRESHOLD_MIN} à {WHEEL_THRESHOLD_MAX}."
			onchange={(autoThreshold) => onwheel({ autoThreshold })}
		/>
	{/if}

	<Switch checked={wheel.natural} onchange={(natural) => onwheel({ natural })}>
		Défilement naturel (le contenu suit le doigt)
	</Switch>
	<Switch checked={wheel.hiRes} onchange={(hiRes) => onwheel({ hiRes })}>
		Défilement haute résolution (plus fin)
	</Switch>

	{#if hasThumbWheel}
		<IntegerField
			label="Vitesse de la molette du pouce"
			value={thumbWheel.speed}
			min={THUMB_SPEED_MIN}
			max={THUMB_SPEED_MAX}
			unit=""
			hint="De {THUMB_SPEED_MIN} (lente) à {THUMB_SPEED_MAX} (rapide)."
			error="Entrez un nombre entier de {THUMB_SPEED_MIN} à {THUMB_SPEED_MAX}."
			onchange={(speed) => onthumbwheel({ speed })}
		/>
		<Switch checked={thumbWheel.invert} onchange={(invert) => onthumbwheel({ invert })}>
			Inverser le sens de la molette du pouce
		</Switch>
	{/if}
</div>
