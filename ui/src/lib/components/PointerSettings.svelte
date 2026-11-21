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
	import { WHEEL_MODES } from '#lib/deck/mouse-labels.js';
	import type { PointerSettings, ThumbWheelSettings, WheelSettings } from '#lib/generated/index.js';
	import RangeControl from './RangeControl.svelte';
	import SegmentedControl from './SegmentedControl.svelte';
	import SettingRow from './SettingRow.svelte';
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
</script>

<div class="max-w-4xl">
	<SettingRow
		label="Sensibilité du pointeur"
		description="Points par pouce. Plus c’est haut, plus le pointeur va loin."
	>
		<RangeControl
			label="Sensibilité du pointeur"
			value={pointer.dpi}
			min={DPI_MIN}
			max={DPI_MAX}
			step={DPI_STEP}
			unit="dpi"
			onchange={onpointer}
		/>
	</SettingRow>

	<SettingRow
		label="Molette principale"
		description="Cran par cran, libre, ou automatique : la molette passe en libre quand on la lance vite."
	>
		<SegmentedControl
			label="Mode de la molette"
			options={WHEEL_MODES}
			value={wheel.mode}
			onchange={(mode) => onwheel({ mode })}
		/>
	</SettingRow>

	{#if wheel.mode === 'auto'}
		<SettingRow
			label="Seuil du mode automatique"
			description="Vitesse à partir de laquelle la molette devient libre."
		>
			<RangeControl
				label="Seuil du mode automatique"
				value={wheel.autoThreshold}
				min={WHEEL_THRESHOLD_MIN}
				max={WHEEL_THRESHOLD_MAX}
				onchange={(autoThreshold) => onwheel({ autoThreshold })}
			/>
		</SettingRow>
	{/if}

	<SettingRow
		label="Défilement naturel"
		description="Le contenu suit le geste, comme sur un écran tactile."
	>
		<Switch
			checked={wheel.natural}
			label="Défilement naturel"
			onchange={(natural) => onwheel({ natural })}
		/>
	</SettingRow>

	<SettingRow
		label="Défilement haute résolution"
		description="Un défilement plus fin et plus fluide dans les applications compatibles."
	>
		<Switch
			checked={wheel.hiRes}
			label="Défilement haute résolution"
			onchange={(hiRes) => onwheel({ hiRes })}
		/>
	</SettingRow>

	{#if hasThumbWheel}
		<SettingRow label="Molette du pouce" description="Vitesse et sens du défilement horizontal.">
			<RangeControl
				label="Vitesse de la molette du pouce"
				value={thumbWheel.speed}
				min={THUMB_SPEED_MIN}
				max={THUMB_SPEED_MAX}
				onchange={(speed) => onthumbwheel({ speed })}
			/>
			<Switch checked={thumbWheel.invert} onchange={(invert) => onthumbwheel({ invert })}>
				Inverser
			</Switch>
		</SettingRow>
	{/if}
</div>
