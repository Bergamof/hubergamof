import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import BatteryBadge from './BatteryBadge.svelte';

describe('BatteryBadge', () => {
	it('shows the charge with the battery icon', async () => {
		const { container } = await render(BatteryBadge, {
			battery: { percent: 78, charging: false }
		});
		await expect.element(page.getByText(/78/)).toBeVisible();
		expect(container.querySelector('svg')).not.toBeNull();
	});

	it('says a low battery in words for the screen reader, and on the screen when labelled', async () => {
		const quiet = await render(BatteryBadge, { battery: { percent: 14, charging: false } });
		await expect.element(page.getByText(', faible')).toBeInTheDocument();
		quiet.unmount();
		await render(BatteryBadge, { battery: { percent: 14, charging: false }, labelled: true });
		await expect.element(page.getByText('· faible')).toBeVisible();
	});

	it('says charging, and does not call a charging battery low', async () => {
		await render(BatteryBadge, { battery: { percent: 10, charging: true }, labelled: true });
		await expect.element(page.getByText('· en charge')).toBeVisible();
		await expect.element(page.getByText('faible')).not.toBeInTheDocument();
	});

	it('says what is asked when the battery is not known, and nothing otherwise', async () => {
		const { container } = await render(BatteryBadge, { battery: undefined, unknown: 'Non lu' });
		await expect.element(page.getByText('Non lu')).toBeVisible();
		expect(container.querySelector('svg')).toBeNull();
		const empty = await render(BatteryBadge, { battery: undefined });
		expect(empty.container.textContent?.trim()).toBe('');
	});
});
