import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { DeviceTheme, StripConfig } from '#lib/generated/index.js';
import type { DialSummary } from '#lib/deck/display.js';
import StripPreview from './StripPreview.svelte';

const STRIP: StripConfig = {
	showSeconds: true,
	showPomodoroProgress: true,
	systemRefreshSeconds: 2,
	showHistory: true,
	loadAlertPercent: 85
};

const DIALS: readonly DialSummary[] = [
	{ name: 'Navigation', rotate: 'Navigation', press: 'Retour', assigned: true, missing: false },
	{ name: 'Rien', rotate: null, press: null, assigned: false, missing: false }
];

async function renderStrip(
	options: { strip?: Partial<StripConfig>; theme?: DeviceTheme } = {},
	selected = false
) {
	const { strip = {}, theme = 'light' } = options;
	const onselect = vi.fn();
	const { container } = await render(StripPreview, {
		strip: { ...STRIP, ...strip },
		theme,
		dials: DIALS,
		selected,
		onselect
	});
	return { container, onselect, svg: container.querySelector('svg') as SVGSVGElement };
}

const drawn = (svg: SVGSVGElement, text: string) =>
	[...svg.querySelectorAll('text')].some((node) => node.textContent?.includes(text));

describe('StripPreview', () => {
	it('is a button that opens the options and tells it is only a preview', async () => {
		const { onselect } = await renderStrip();
		const button = page.getByRole('button', { name: /Bande tactile : aperçu/ });
		await button.click();
		expect(onselect).toHaveBeenCalledOnce();
	});

	it('marks itself current when selected', async () => {
		await renderStrip({}, true);
		await expect
			.element(page.getByRole('button', { name: /Bande tactile/ }))
			.toHaveAttribute('aria-current', 'true');
	});

	it('shows sample figures, hidden from assistive technology', async () => {
		const { svg } = await renderStrip();
		expect(svg.getAttribute('aria-hidden')).toBe('true');
		for (const text of [
			'14:37',
			'CPU',
			'RAM',
			'SWAP',
			'Docker › api-server',
			'Focus 2/4',
			'18:24'
		]) {
			expect(drawn(svg, text), text).toBe(true);
		}
	});

	it('reminds what each dial does, under the strip', async () => {
		const { svg } = await renderStrip();
		expect(drawn(svg, 'Navigation')).toBe(true);
		expect(drawn(svg, 'Retour')).toBe(true);
	});

	it('follows the theme', async () => {
		const light = await renderStrip({ theme: 'light' });
		expect(light.svg.getAttribute('data-theme-preview')).toBe('light');
		const lightBackground = light.svg.querySelector('rect')?.getAttribute('fill');
		const dark = await renderStrip({ theme: 'dark' });
		expect(dark.svg.getAttribute('data-theme-preview')).toBe('dark');
		expect(dark.svg.querySelector('rect')?.getAttribute('fill')).not.toBe(lightBackground);
	});

	it('leaves out the seconds, the progress bar and the curves on request', async () => {
		const full = await renderStrip();
		expect(drawn(full.svg, '42')).toBe(true);
		expect(full.svg.querySelectorAll('polyline')).toHaveLength(3);
		const rects = full.svg.querySelectorAll('rect').length;

		const bare = await renderStrip({
			strip: { showSeconds: false, showPomodoroProgress: false, showHistory: false }
		});
		expect(drawn(bare.svg, '42')).toBe(false);
		expect(bare.svg.querySelectorAll('polyline')).toHaveLength(0);
		// Two bars per measure instead of the curves, two bars fewer for the progress.
		expect(bare.svg.querySelectorAll('rect').length).toBe(rects - 2 + 6);
	});
});
