import { describe, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import Tabs, { panelId, tabId } from './Tabs.svelte';

const TABS = [
	{ id: 'touches', label: 'Touches' },
	{ id: 'settings', label: 'Pointeur et molette' },
	{ id: 'haptic', label: 'Haptique' }
] as const;

async function renderTabs(selected: 'touches' | 'settings' | 'haptic' = 'touches') {
	const onselect = vi.fn();
	await render(Tabs, { label: 'Réglages', tabs: TABS, selected, prefix: 'm', onselect });
	return onselect;
}

describe('Tabs', () => {
	it('is a tab list with the selected tab marked, and the others out of the tab order', async () => {
		await renderTabs('settings');
		await expect.element(page.getByRole('tablist', { name: 'Réglages' })).toBeVisible();
		const selected = page.getByRole('tab', { name: 'Pointeur et molette' });
		await expect.element(selected).toHaveAttribute('aria-selected', 'true');
		await expect.element(selected).toHaveAttribute('tabindex', '0');
		await expect
			.element(page.getByRole('tab', { name: 'Touches' }))
			.toHaveAttribute('tabindex', '-1');
	});

	it('points each tab to its panel', async () => {
		await renderTabs();
		const tab = page.getByRole('tab', { name: 'Haptique' });
		await expect.element(tab).toHaveAttribute('id', tabId('m', 'haptic'));
		await expect.element(tab).toHaveAttribute('aria-controls', panelId('m', 'haptic'));
	});

	it('selects the tab clicked', async () => {
		const onselect = await renderTabs();
		await page.getByRole('tab', { name: 'Haptique' }).click();
		expect(onselect).toHaveBeenCalledExactlyOnceWith('haptic');
	});

	it('moves with the arrow keys, around the ends, and with Home and End', async () => {
		const onselect = await renderTabs();
		(page.getByRole('tab', { name: 'Touches' }).element() as HTMLElement).focus();
		await userEvent.keyboard('{ArrowRight}');
		expect(onselect).toHaveBeenLastCalledWith('settings');
		await userEvent.keyboard('{ArrowLeft}');
		expect(onselect).toHaveBeenLastCalledWith('haptic');
		await userEvent.keyboard('{End}');
		expect(onselect).toHaveBeenLastCalledWith('haptic');
		await userEvent.keyboard('{Home}');
		expect(onselect).toHaveBeenLastCalledWith('touches');
	});

	it('leaves other keys alone', async () => {
		const onselect = await renderTabs();
		(page.getByRole('tab', { name: 'Touches' }).element() as HTMLElement).focus();
		await userEvent.keyboard('a');
		expect(onselect).not.toHaveBeenCalled();
	});
});
