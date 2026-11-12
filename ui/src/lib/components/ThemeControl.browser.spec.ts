import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import ThemeControl from './ThemeControl.svelte';

describe('ThemeControl', () => {
	it('shows the theme of the device', async () => {
		await render(ThemeControl, { theme: 'dark', onchange: vi.fn() });
		await expect.element(page.getByRole('radio', { name: 'Sombre' })).toBeChecked();
		await expect.element(page.getByRole('radio', { name: 'Clair' })).not.toBeChecked();
		await expect.element(page.getByRole('group', { name: 'Thème de l’appareil' })).toBeVisible();
	});

	it('reports the chosen theme at once', async () => {
		const onchange = vi.fn();
		await render(ThemeControl, { theme: 'light', onchange });
		await page.getByRole('radio', { name: 'Sombre' }).click();
		expect(onchange).toHaveBeenLastCalledWith('dark');
	});
});
