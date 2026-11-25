import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ManagerApi } from '#lib/api/manager-api.js';
import BackupExport from './BackupExport.svelte';

async function renderExport(exportBackup: ManagerApi['exportBackup']) {
	await render(BackupExport, { api: { exportBackup } as ManagerApi });
}

const SAVED = { ok: true, value: new Blob(['PK']) };

describe('BackupExport', () => {
	it('warns that the file is private before it is saved', async () => {
		await renderExport(vi.fn());
		await expect.element(page.getByText('Gardez ce fichier privé.')).toBeVisible();
		await expect.element(page.getByText(/identité Git/)).toBeVisible();
	});

	it('exports everything by default and says where the file went', async () => {
		const exportBackup = vi.fn().mockResolvedValue(SAVED);
		await renderExport(exportBackup);
		for (const name of ['Stream Deck', 'Souris', 'Menu circulaire', 'Modules', 'Git', 'SSH']) {
			await expect.element(page.getByRole('checkbox', { name })).toBeChecked();
		}
		await page.getByRole('button', { name: 'Exporter la configuration' }).click();
		expect(exportBackup).toHaveBeenCalledWith(undefined);
		await expect.element(page.getByText(/Sauvegarde exportée/)).toBeVisible();
	});

	it('limits the export to what is ticked', async () => {
		const exportBackup = vi.fn().mockResolvedValue(SAVED);
		await renderExport(exportBackup);
		await page.getByRole('checkbox', { name: 'Git' }).click();
		await page.getByRole('checkbox', { name: 'SSH' }).click();
		await page.getByRole('button', { name: 'Exporter la configuration' }).click();
		expect(exportBackup).toHaveBeenCalledWith([
			'devices',
			'mice',
			'radial-menu',
			'modules',
			'machine'
		]);
	});

	it('exports nothing when nothing is ticked, and says why', async () => {
		const exportBackup = vi.fn();
		await renderExport(exportBackup);
		for (const checkbox of page.getByRole('checkbox').all()) await checkbox.click();
		const button = page.getByRole('button', { name: 'Exporter la configuration' });
		await expect.element(page.getByText('Cochez au moins une case.')).toBeVisible();
		await expect.element(button).toHaveAttribute('aria-disabled', 'true');
		await button.click({ force: true });
		expect(exportBackup).not.toHaveBeenCalled();
	});

	it('shows a refusal instead of saving it', async () => {
		const exportBackup = vi.fn().mockResolvedValue({
			ok: false,
			problem: { text: 'L’ordinateur est verrouillé.', detail: null }
		});
		await renderExport(exportBackup);
		await page.getByRole('button', { name: 'Exporter la configuration' }).click();
		await expect.element(page.getByText(/Sauvegarde non exportée/)).toBeVisible();
		await expect.element(page.getByText(/Sauvegarde exportée :/)).not.toBeInTheDocument();
	});
});
