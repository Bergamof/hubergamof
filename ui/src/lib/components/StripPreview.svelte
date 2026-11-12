<!--
@component
Preview of the touch strip, with sample figures (not the live strip): it follows the theme and the
options of the strip, and shows under it what each dial does. Selecting it opens the options.
The palettes and the layout copy those of `crates/manager/src/render/strip.rs`: change both together.
-->
<script lang="ts">
	import type { StripConfig } from '#lib/generated/index.js';
	import type { DialSummary } from '#lib/deck/display.js';

	interface Props {
		strip: StripConfig;
		/** The dials, left to right: the reminders under the strip. */
		dials: readonly DialSummary[];
		selected: boolean;
		onselect: () => void;
	}

	let { strip, dials, selected, onselect }: Props = $props();

	/** Colours of each theme, as the manager draws the strip. */
	const PALETTES = {
		light: {
			background: '#e7ebf1',
			ink: '#13213b',
			muted: '#566178',
			track: '#cdd5e0',
			line: '#c3ccd9',
			cpu: '#e0502a',
			ram: '#1d7fd6',
			swap: '#7b50d4',
			volume: '#0d8f7b',
			working: '#d83a1f'
		},
		dark: {
			background: '#151a23',
			ink: '#eef2f8',
			muted: '#8e9ab0',
			track: '#2a3342',
			line: '#2c3544',
			cpu: '#ff7a52',
			ram: '#4aa8ff',
			swap: '#a98bff',
			volume: '#34c9ae',
			working: '#ff5a3d'
		}
	} as const;

	const WIDTH = 800;
	const MARGIN = 16;
	const CURVE = { x: 282, width: 170, height: 14 } as const;
	const HINT_CHAR_WIDTH = 7.3;
	const HINT_MAX_CHARS = 13;
	const SAMPLE_VOLUME = 62;
	const SAMPLE_PROGRESS = 0.26;
	const MEASURES = [
		{
			label: 'CPU',
			value: 34,
			color: 'cpu',
			history: [22, 30, 26, 40, 55, 38, 30, 28, 44, 62, 48, 36, 30, 34]
		},
		{
			label: 'RAM',
			value: 61,
			color: 'ram',
			history: [58, 58, 59, 59, 60, 60, 60, 61, 61, 61, 61, 62, 61, 61]
		},
		{ label: 'SWAP', value: 8, color: 'swap', history: [8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8] }
	] as const;

	const palette = $derived(PALETTES[strip.theme]);

	function baseline(row: number): number {
		return 26 + row * 20;
	}

	function curve(history: readonly number[], row: number): string {
		const step = CURVE.width / (history.length - 1);
		return history
			.map(
				(value, index) =>
					`${(CURVE.x + index * step).toFixed(1)},${(baseline(row) - (value / 100) * CURVE.height).toFixed(1)}`
			)
			.join(' ');
	}

	function endPoint(history: readonly number[], row: number): [number, number] {
		const last = history[history.length - 1] ?? 0;
		return [CURVE.x + CURVE.width, baseline(row) - (last / 100) * CURVE.height];
	}

	function shortened(text: string): string {
		return text.length > HINT_MAX_CHARS ? `${text.slice(0, HINT_MAX_CHARS - 1)}…` : text;
	}

	/** Horizontal start of the reminder of dial `index`. */
	function hintX(index: number): number {
		return MARGIN + (index * WIDTH) / Math.max(dials.length, 1);
	}

	/** Where the press part of a reminder starts, after its rotation part. */
	function pressX(index: number, rotate: string | null): number {
		const start = hintX(index);
		return rotate ? start + 16 + shortened(rotate).length * HINT_CHAR_WIDTH + 10 : start;
	}
</script>

<button
	type="button"
	aria-current={selected ? 'true' : undefined}
	class="block w-full rounded-2xl p-1 hover:bg-device-key focus-visible:outline-3 focus-visible:outline-offset-2 focus-visible:outline-device-ring aria-current:bg-device-key aria-current:ring-2 aria-current:ring-accent"
	onclick={onselect}
>
	<span class="sr-only">Bande tactile : aperçu avec des valeurs d’exemple, ouvre ses réglages</span>
	<svg
		viewBox="0 0 {WIDTH} 100"
		aria-hidden="true"
		data-theme-preview={strip.theme}
		class="block w-full rounded-lg"
		font-family="inherit"
		font-weight="600"
	>
		<rect width={WIDTH} height="100" fill={palette.background} />

		<!-- Heure, secondes et date -->
		<text x="18" y="54" font-size="56" letter-spacing="-1.5" fill={palette.ink}>14:37</text>
		{#if strip.showSeconds}
			<text x="182" y="54" font-size="22" fill={palette.muted}>42</text>
		{/if}
		<text x="20" y="71" font-size="13" letter-spacing="1" fill={palette.muted}
			>JEU. 12 NOV. · SEM. 46</text
		>
		<rect x="224" y="12" width="1" height="58" fill={palette.line} />

		<!-- Mesures du système -->
		{#each MEASURES as measure, row (measure.label)}
			{@const color = palette[measure.color]}
			<text x="240" y={baseline(row)} font-size="12" letter-spacing="1" fill={palette.muted}
				>{measure.label}</text
			>
			{#if strip.showHistory}
				{@const [endX, endY] = endPoint(measure.history, row)}
				<polyline
					points={curve(measure.history, row)}
					fill="none"
					stroke={color}
					stroke-width="2"
					stroke-linejoin="round"
					stroke-linecap="round"
				/>
				<circle cx={endX} cy={endY} r="2.8" fill={color} />
			{:else}
				<rect
					x={CURVE.x}
					y={baseline(row) - 7}
					width={CURVE.width}
					height="5"
					rx="2.5"
					fill={palette.track}
				/>
				<rect
					x={CURVE.x}
					y={baseline(row) - 7}
					width={(CURVE.width * measure.value) / 100}
					height="5"
					rx="2.5"
					fill={color}
				/>
			{/if}
			<text x="508" y={baseline(row) + 1} font-size="15" text-anchor="end" fill={palette.ink}>
				{measure.value}<tspan font-size="10" fill={palette.muted}> %</tspan>
			</text>
		{/each}
		<rect x="528" y="12" width="1" height="58" fill={palette.line} />

		<!-- Page, volume et pomodoro -->
		<text x="546" y="21" font-size="14" fill={palette.ink}>Docker › api-server</text>
		{#each [0, 1, 2] as dot (dot)}
			<circle cx={766 + dot * 12} cy="17" r="3.2" fill={dot === 1 ? palette.ink : palette.track} />
		{/each}
		<rect x="566" y="35" width="186" height="3" rx="1.5" fill={palette.track} />
		<rect
			x="566"
			y="35"
			width={(186 * SAMPLE_VOLUME) / 100}
			height="3"
			rx="1.5"
			fill={palette.volume}
		/>
		<text x="546" y="40" font-size="11" fill={palette.muted}>♪</text>
		<text x="782" y="40" font-size="11" text-anchor="end" fill={palette.muted}>{SAMPLE_VOLUME}</text
		>
		<circle cx="550.5" cy="56" r="4.5" fill={palette.working} />
		<text x="562" y="60" font-size="12" fill={palette.muted}>Focus 2/4</text>
		<text x="782" y="63" font-size="26" text-anchor="end" fill={palette.ink}>18:24</text>
		{#if strip.showPomodoroProgress}
			<rect x={MARGIN} y="75" width={WIDTH - 2 * MARGIN} height="4" rx="2" fill={palette.track} />
			<rect
				x={MARGIN}
				y="75"
				width={(WIDTH - 2 * MARGIN) * SAMPLE_PROGRESS}
				height="4"
				rx="2"
				fill={palette.working}
			/>
		{/if}

		<!-- Rappels des molettes -->
		<rect x={MARGIN} y="83.5" width={WIDTH - 2 * MARGIN} height="1" fill={palette.line} />
		{#each dials as dial, index (index)}
			{#if dial.rotate}
				<g
					transform="translate({hintX(index)} 85.5) scale(0.5)"
					fill="none"
					stroke={palette.muted}
					stroke-width="2.6"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<path d="M20 12a8 8 0 1 1-2.5-5.8" />
					<path d="M20 4v5h-5" />
				</g>
				<text x={hintX(index) + 16} y="96" font-size="13" fill={palette.muted}
					>{shortened(dial.rotate)}</text
				>
			{/if}
			{#if dial.press}
				<g
					transform="translate({pressX(index, dial.rotate)} 85.5) scale(0.5)"
					stroke={palette.muted}
					stroke-width="2.6"
				>
					<circle cx="12" cy="12" r="9" fill="none" />
					<circle cx="12" cy="12" r="3.2" fill={palette.muted} />
				</g>
				<text x={pressX(index, dial.rotate) + 16} y="96" font-size="13" fill={palette.muted}
					>{shortened(dial.press)}</text
				>
			{/if}
		{/each}
	</svg>
</button>
