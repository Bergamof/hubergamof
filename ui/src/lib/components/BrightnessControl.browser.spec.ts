import { describe, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import BrightnessControl from './BrightnessControl.svelte';

describe('BrightnessControl', () => {
	it('shows the brightness on a labelled slider', async () => {
		await render(BrightnessControl, { brightness: 40, onchange: vi.fn() });
		const slider = page.getByLabelText('Luminosité');
		await expect.element(slider).toHaveValue('40');
		await expect.element(slider).toHaveAttribute('aria-valuetext', '40 %');
	});

	it('reports the value chosen with the keyboard', async () => {
		const onchange = vi.fn();
		await render(BrightnessControl, { brightness: 40, onchange });
		const slider = page.getByLabelText('Luminosité');
		await slider.click();
		await userEvent.keyboard('{ArrowRight}');
		await expect.poll(() => onchange.mock.calls.length).toBeGreaterThan(0);
		expect(onchange.mock.lastCall?.[0]).toBeGreaterThan(40);
	});
});
