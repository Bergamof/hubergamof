import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import PointerSettings from './PointerSettings.svelte';
import { mouseConfig } from './mouse-fixtures.js';

async function renderSettings(
	overrides: { hasThumbWheel?: boolean; mode?: 'ratchet' | 'free' | 'auto' } = {}
) {
	const config = mouseConfig();
	const handlers = { onpointer: vi.fn(), onwheel: vi.fn(), onthumbwheel: vi.fn() };
	await render(PointerSettings, {
		pointer: config.pointer,
		wheel: { ...config.wheel, mode: overrides.mode ?? 'ratchet' },
		thumbWheel: config.thumbWheel,
		hasThumbWheel: overrides.hasThumbWheel ?? true,
		...handlers
	});
	return handlers;
}

describe('PointerSettings', () => {
	it('shows the sensitivity with its unit', async () => {
		await renderSettings();
		const slider = page.getByLabelText('Sensibilité du pointeur');
		await expect.element(slider).toHaveValue('1600');
		await expect.element(slider).toHaveAttribute('aria-valuetext', '1600 dpi');
	});

	it('sends the sensitivity chosen', async () => {
		const { onpointer } = await renderSettings();
		await page.getByLabelText('Sensibilité du pointeur').fill('2000');
		expect(onpointer).toHaveBeenLastCalledWith(2000);
	});

	it('changes the mode of the wheel and shows the threshold of the automatic mode only', async () => {
		const { onwheel } = await renderSettings();
		expect(document.body.textContent).not.toContain('Seuil de vitesse');
		await page.getByLabelText('Molette', { exact: true }).selectOptions('auto');
		expect(onwheel).toHaveBeenCalledExactlyOnceWith({ mode: 'auto' });
	});

	it('changes the threshold in the automatic mode', async () => {
		const { onwheel } = await renderSettings({ mode: 'auto' });
		await page.getByLabelText('Seuil de vitesse du mode automatique').fill('30');
		expect(onwheel).toHaveBeenLastCalledWith({ autoThreshold: 30 });
	});

	it('tells an invalid threshold and does not send it', async () => {
		const { onwheel } = await renderSettings({ mode: 'auto' });
		await page.getByLabelText('Seuil de vitesse du mode automatique').fill('99');
		expect(onwheel).not.toHaveBeenCalled();
		await expect.element(page.getByRole('alert')).toBeVisible();
	});

	it('switches the direction and the resolution of the scroll', async () => {
		const { onwheel } = await renderSettings();
		await page.getByRole('checkbox', { name: /Défilement naturel/ }).click();
		expect(onwheel).toHaveBeenLastCalledWith({ natural: true });
		await page.getByRole('checkbox', { name: /haute résolution/ }).click();
		expect(onwheel).toHaveBeenLastCalledWith({ hiRes: false });
	});

	it('configures the thumb wheel of a model that has one', async () => {
		const { onthumbwheel } = await renderSettings();
		await page.getByLabelText('Vitesse de la molette du pouce').fill('8');
		expect(onthumbwheel).toHaveBeenLastCalledWith({ speed: 8 });
		await page.getByRole('checkbox', { name: /Inverser/ }).click();
		expect(onthumbwheel).toHaveBeenLastCalledWith({ invert: true });
	});

	it('leaves the thumb wheel out of a model without one', async () => {
		await renderSettings({ hasThumbWheel: false });
		expect(document.body.textContent).not.toContain('molette du pouce');
	});
});
