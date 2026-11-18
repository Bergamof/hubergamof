import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ManagerApi } from '#lib/api/manager-api.js';
import type { DeviceInfo } from '#lib/generated/index.js';
import ExportImport from './ExportImport.svelte';

const DEVICE: DeviceInfo = {
	serial: 'DECK',
	model: 'virtual',
	displayName: 'Stream Deck de test',
	capabilities: {
		keyColumns: 4,
		keyRows: 2,
		keyImageSize: { width: 120, height: 120 },
		dialCount: 4,
		touchStrip: null
	}
};

const ARCHIVE = new File(['zip'], 'sauvegarde.zip', { type: 'application/zip' });

async function renderImport(importArchive: ManagerApi['importArchive']) {
	const { container } = await render(ExportImport, {
		device: DEVICE,
		api: { importArchive } as ManagerApi
	});
	const input = container.querySelector('input[type="file"]') as HTMLInputElement;
	return { input };
}

/** Picks `file` as the file chooser would. */
function pick(input: HTMLInputElement, file: File) {
	const files = new DataTransfer();
	files.items.add(file);
	input.files = files.files;
	input.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('ExportImport', () => {
	it('offers the export as a download of the manager', async () => {
		await renderImport(vi.fn());
		const link = page.getByRole('link', { name: 'Exporter la configuration' });
		await expect.element(link).toBeVisible();
		const element = link.element();
		expect(element.getAttribute('href')).toBe('/api/devices/DECK/export');
		expect(element.getAttribute('rel')).toBe('external');
	});

	it('asks for a confirmation naming the file before replacing, and imports nothing if cancelled', async () => {
		const importArchive = vi.fn();
		const { input } = await renderImport(importArchive);
		pick(input, ARCHIVE);
		await expect.element(page.getByRole('dialog')).toBeVisible();
		await expect.element(page.getByText(/sauvegarde\.zip/)).toBeVisible();
		await page.getByRole('button', { name: 'Annuler' }).click();
		expect(importArchive).not.toHaveBeenCalled();
	});

	it('imports the confirmed archive and says so', async () => {
		const importArchive = vi.fn().mockResolvedValue({ ok: true, value: undefined });
		const { input } = await renderImport(importArchive);
		pick(input, ARCHIVE);
		await page.getByRole('button', { name: 'Remplacer' }).click();
		expect(importArchive).toHaveBeenCalledWith('DECK', ARCHIVE);
		await expect.element(page.getByText(/Configuration importée depuis/)).toBeVisible();
	});

	it('shows why an archive was refused', async () => {
		const importArchive = vi
			.fn()
			.mockResolvedValue({ ok: false, problem: { text: 'Archive invalide.', detail: 'bad zip' } });
		const { input } = await renderImport(importArchive);
		pick(input, ARCHIVE);
		await page.getByRole('button', { name: 'Remplacer' }).click();
		await expect.element(page.getByText(/Configuration non importée/)).toBeVisible();
		await expect.element(page.getByText('bad zip')).toBeVisible();
	});
});
