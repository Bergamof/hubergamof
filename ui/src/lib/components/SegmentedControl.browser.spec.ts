import { describe, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import SegmentedControl from './SegmentedControl.svelte';

const OPTIONS = [
	{ value: 'low', label: 'Faible' },
	{ value: 'medium', label: 'Moyen' },
	{ value: 'high', label: 'Fort' }
] as const;

describe('SegmentedControl', () => {
	it('is a radio group named for what it chooses, showing the value', async () => {
		await render(SegmentedControl, {
			label: 'Intensité',
			options: OPTIONS,
			value: 'medium',
			onchange: vi.fn()
		});
		await expect.element(page.getByRole('radiogroup', { name: 'Intensité' })).toBeVisible();
		await expect.element(page.getByRole('radio', { name: 'Moyen' })).toBeChecked();
		await expect.element(page.getByRole('radio', { name: 'Fort' })).not.toBeChecked();
	});

	it('reports the segment chosen', async () => {
		const onchange = vi.fn();
		await render(SegmentedControl, {
			label: 'Intensité',
			options: OPTIONS,
			value: 'low',
			onchange
		});
		await page.getByRole('radio', { name: 'Fort' }).click();
		expect(onchange).toHaveBeenCalledExactlyOnceWith('high');
	});

	it('is driven by the arrow keys like any radio group', async () => {
		const onchange = vi.fn();
		await render(SegmentedControl, {
			label: 'Intensité',
			options: OPTIONS,
			value: 'low',
			onchange
		});
		(page.getByRole('radio', { name: 'Faible' }).element() as HTMLElement).focus();
		await userEvent.keyboard('{ArrowRight}');
		expect(onchange).toHaveBeenLastCalledWith('medium');
	});
});
