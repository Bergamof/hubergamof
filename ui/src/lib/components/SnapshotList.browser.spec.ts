import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ManagerApi } from '#lib/api/manager-api.js';
import type { ImportReport, SnapshotInfo } from '#lib/generated/index.js';
import SnapshotList from './SnapshotList.svelte';

const SNAPSHOT: SnapshotInfo = {
	id: '1-0',
	createdAt: Date.UTC(2026, 10, 25, 12, 0) / 1000,
	appVersion: '1.2.3',
	sections: ['git', 'ssh']
};
const PREVIEW = {
	sections: [{ section: 'git', change: 'replaced' as const, present: true, executables: [] }],
	kept: []
};
const REPORT: ImportReport = {
	snapshot: '2-0',
	sections: [{ section: 'git', outcome: { result: 'applied' } }]
};

interface Mocks {
	listSnapshots?: ManagerApi['listSnapshots'];
	previewSnapshot?: ManagerApi['previewSnapshot'];
	restoreSnapshot?: ManagerApi['restoreSnapshot'];
}

async function renderList(mocks: Mocks, onchange = vi.fn()) {
	const api = {
		listSnapshots:
			mocks.listSnapshots ?? vi.fn().mockResolvedValue({ ok: true, value: [SNAPSHOT] }),
		previewSnapshot:
			mocks.previewSnapshot ?? vi.fn().mockResolvedValue({ ok: true, value: PREVIEW }),
		restoreSnapshot: mocks.restoreSnapshot ?? vi.fn().mockResolvedValue({ ok: true, value: REPORT })
	} as unknown as ManagerApi;
	const screen = await render(SnapshotList, { api, names: new Map(), version: 0, onchange });
	return { api, onchange, ...screen };
}

describe('SnapshotList', () => {
	it('lists the snapshots with their date, version and sections', async () => {
		await renderList({});
		await expect.element(page.getByText(/novembre 2026/).first()).toBeVisible();
		await expect.element(page.getByText(/Version 1\.2\.3/)).toBeVisible();
		await expect.element(page.getByText(/Git, SSH/)).toBeVisible();
	});

	it('says when there is none', async () => {
		await renderList({ listSnapshots: vi.fn().mockResolvedValue({ ok: true, value: [] }) });
		await expect
			.element(page.getByText('Aucune sauvegarde automatique pour l’instant.'))
			.toBeVisible();
	});

	it('shows why the list could not be read', async () => {
		await renderList({
			listSnapshots: vi.fn().mockResolvedValue({
				ok: false,
				problem: { text: 'Gestionnaire injoignable.', detail: null }
			})
		});
		await expect.element(page.getByText(/Gestionnaire injoignable/)).toBeVisible();
	});

	it('shows what a restore would do before it restores, and restores nothing if cancelled', async () => {
		const restoreSnapshot = vi.fn();
		const { api } = await renderList({ restoreSnapshot });
		await page.getByRole('button', { name: /Restaurer la sauvegarde du/ }).click();
		expect(api.previewSnapshot).toHaveBeenCalledWith('1-0');
		await expect.element(page.getByRole('dialog')).toBeVisible();
		await expect.element(page.getByText('Remplacée')).toBeVisible();
		await page.getByRole('button', { name: 'Annuler' }).click();
		expect(restoreSnapshot).not.toHaveBeenCalled();
	});

	it('restores the snapshot once confirmed and says what became of each section', async () => {
		const restoreSnapshot = vi.fn().mockResolvedValue({ ok: true, value: REPORT });
		const { onchange } = await renderList({ restoreSnapshot });
		await page.getByRole('button', { name: /Restaurer la sauvegarde du/ }).click();
		await page.getByRole('dialog').getByRole('button', { name: 'Restaurer' }).click();
		expect(restoreSnapshot).toHaveBeenCalledWith('1-0');
		await expect.element(page.getByText('Appliquée')).toBeVisible();
		expect(onchange).toHaveBeenCalledOnce();
	});

	it('shows a refusal to restore', async () => {
		const restoreSnapshot = vi.fn().mockResolvedValue({
			ok: false,
			problem: { text: 'L’ordinateur est verrouillé.', detail: null }
		});
		await renderList({ restoreSnapshot });
		await page.getByRole('button', { name: /Restaurer la sauvegarde du/ }).click();
		await page.getByRole('dialog').getByRole('button', { name: 'Restaurer' }).click();
		await expect.element(page.getByText(/Sauvegarde automatique non restaurée/)).toBeVisible();
	});
});
