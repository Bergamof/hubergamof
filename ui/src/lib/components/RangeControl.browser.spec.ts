import { afterEach, describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import RangeControl from './RangeControl.svelte';

afterEach(() => {
	vi.useRealTimers();
});

describe('RangeControl', () => {
	it('shows the value with its unit, for the eye and for assistive technologies', async () => {
		await render(RangeControl, {
			label: 'Sensibilité',
			value: 1600,
			min: 200,
			max: 8000,
			step: 50,
			unit: 'dpi',
			onchange: vi.fn()
		});
		const slider = page.getByLabelText('Sensibilité');
		await expect.element(slider).toHaveValue('1600');
		await expect.element(slider).toHaveAttribute('aria-valuetext', '1600 dpi');
		await expect.element(page.getByText('1600 dpi')).toBeInTheDocument();
	});

	it('shows a value without a unit as it is', async () => {
		await render(RangeControl, { label: 'Vitesse', value: 5, min: 1, max: 10, onchange: vi.fn() });
		await expect.element(page.getByLabelText('Vitesse')).toHaveAttribute('aria-valuetext', '5');
	});

	it('sends the value it is let go on', async () => {
		const onchange = vi.fn();
		await render(RangeControl, { label: 'Vitesse', value: 5, min: 1, max: 10, onchange });
		await page.getByLabelText('Vitesse').fill('8');
		expect(onchange).toHaveBeenLastCalledWith(8);
	});
});
