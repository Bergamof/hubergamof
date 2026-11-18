import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import UpdatePanel from './UpdatePanel.svelte';

describe('UpdatePanel', () => {
	it('announces a newer release with a safe link to its notes', async () => {
		await render(UpdatePanel, {
			version: '1.0.0',
			status: { type: 'available', version: '1.2.0', notesUrl: 'https://example.org/notes' },
			oncheck: vi.fn()
		});
		await expect.element(page.getByText('Version 1.2.0 disponible.')).toBeVisible();
		await expect.element(page.getByText(/Vous utilisez la 1\.0\.0/)).toBeVisible();
		const link = page.getByRole('link', { name: /Notes de version/ });
		await expect.element(link).toHaveAttribute('href', 'https://example.org/notes');
		await expect.element(link).toHaveAttribute('target', '_blank');
		await expect.element(link).toHaveAttribute('rel', 'noopener noreferrer');
	});

	it.each([
		['upToDate' as const, {}, /HuBergamof est à jour \(version 1\.0\.0\)/],
		['checking' as const, {}, /Recherche de mises à jour…/],
		['checkFailed' as const, { message: 'hors ligne' }, /a échoué : hors ligne/],
		['unknown' as const, {}, /Version 1\.0\.0\. Aucune recherche/]
	])('shows the %s state', async (type, extra, text) => {
		await render(UpdatePanel, {
			version: '1.0.0',
			status: { type, ...extra } as never,
			oncheck: vi.fn()
		});
		await expect.element(page.getByText(text)).toBeVisible();
	});

	it('copes with an unknown running version', async () => {
		await render(UpdatePanel, {
			version: undefined,
			status: { type: 'upToDate' },
			oncheck: vi.fn()
		});
		await expect.element(page.getByText('HuBergamof est à jour.')).toBeVisible();
	});

	it('asks for a check, except while one is running', async () => {
		const oncheck = vi.fn();
		const view = await render(UpdatePanel, {
			version: '1.0.0',
			status: { type: 'upToDate' },
			oncheck
		});
		await page.getByRole('button', { name: 'Rechercher des mises à jour' }).click();
		expect(oncheck).toHaveBeenCalledTimes(1);

		await view.rerender({ version: '1.0.0', status: { type: 'checking' }, oncheck });
		const button = page.getByRole('button', { name: 'Rechercher des mises à jour' });
		await expect.element(button).toHaveAttribute('aria-disabled', 'true');
		await button.click({ force: true });
		expect(oncheck).toHaveBeenCalledTimes(1);
	});
});
