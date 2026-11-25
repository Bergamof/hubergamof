import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ManagerApi } from '#lib/api/manager-api.js';
import type { BackupPreview, DeviceInfo, ImportReport } from '#lib/generated/index.js';
import BackupImport from './BackupImport.svelte';

const NEW_DECK: DeviceInfo = {
	serial: 'NEW',
	model: 'virtual',
	displayName: 'Stream Deck neuf',
	capabilities: {
		keyColumns: 4,
		keyRows: 2,
		keyImageSize: { width: 120, height: 120 },
		dialCount: 4,
		touchStrip: null
	}
};

const PREVIEW: BackupPreview = {
	sections: [
		{ section: 'devices/OLD', change: 'added', present: false, executables: [] },
		{
			section: 'machine',
			change: 'replaced',
			present: true,
			executables: [
				{ kind: 'shellAlias', name: 'gs', detail: 'git status', enabled: false, takesOver: true }
			]
		},
		{ section: 'git', change: 'same', present: true, executables: [] }
	],
	kept: ['mice/MX1']
};

const REPORT: ImportReport = {
	snapshot: '1-0',
	sections: [
		{ section: 'devices/OLD', outcome: { result: 'stored' } },
		{ section: 'machine', outcome: { result: 'notApplied', message: 'no home' } }
	]
};

const ZIP = new File(['zip'], 'sauvegarde.zip', { type: 'application/zip' });

interface Mocks {
	previewBackup?: ManagerApi['previewBackup'];
	importBackup?: ManagerApi['importBackup'];
	restoreSnapshot?: ManagerApi['restoreSnapshot'];
}

async function renderImport(mocks: Mocks, onchange = vi.fn()) {
	const api = {
		previewBackup: mocks.previewBackup ?? vi.fn().mockResolvedValue({ ok: true, value: PREVIEW }),
		importBackup: mocks.importBackup ?? vi.fn().mockResolvedValue({ ok: true, value: REPORT }),
		restoreSnapshot: mocks.restoreSnapshot ?? vi.fn().mockResolvedValue({ ok: true, value: REPORT })
	} as unknown as ManagerApi;
	const { container } = await render(BackupImport, {
		api,
		names: new Map([['NEW', 'Stream Deck neuf']]),
		decks: [NEW_DECK],
		onchange
	});
	const input = container.querySelector('input[type="file"]') as HTMLInputElement;
	return { api, input, onchange };
}

/** Picks `file` as the file chooser would. */
function pick(input: HTMLInputElement, file: File) {
	const files = new DataTransfer();
	files.items.add(file);
	input.files = files.files;
	input.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('BackupImport', () => {
	it('shows what the file holds before anything is written, and ticks what changes', async () => {
		const { input, api } = await renderImport({});
		pick(input, ZIP);
		await expect.element(page.getByText('2. Vérifier ce qui sera importé')).toBeVisible();
		await expect.element(page.getByText('sauvegarde.zip')).toBeVisible();
		await expect.element(page.getByRole('checkbox', { name: /Stream Deck OLD/ })).toBeChecked();
		await expect.element(page.getByRole('checkbox', { name: 'Alias du shell' })).toBeChecked();
		await expect.element(page.getByRole('checkbox', { name: 'Git' })).not.toBeChecked();
		await expect.element(page.getByText('Identique')).toBeVisible();
		await expect.element(page.getByText(/sa configuration est gardée/)).toBeVisible();
		await expect.element(page.getByText(/Laissé tel quel/)).toBeVisible();
		expect(api.importBackup).not.toHaveBeenCalled();
	});

	it('lists what would run on this computer, enabled or not, and what it takes over', async () => {
		const { input } = await renderImport({});
		pick(input, ZIP);
		const table = page.getByRole('table');
		await expect.element(table).toBeVisible();
		await expect.element(table.getByText('git status')).toBeVisible();
		await expect.element(table.getByText(/Désactivé/)).toBeVisible();
		await expect.element(table.getByText(/Reprend « gs » déjà présent/)).toBeVisible();
	});

	it('imports the sections ticked, says what became of each and tells the list of snapshots', async () => {
		const importBackup = vi.fn().mockResolvedValue({ ok: true, value: REPORT });
		const { input, onchange } = await renderImport({ importBackup });
		pick(input, ZIP);
		await page.getByRole('checkbox', { name: /Stream Deck OLD/ }).click();
		await page.getByRole('button', { name: 'Importer 1 section' }).click();
		expect(importBackup).toHaveBeenCalledOnce();
		const [sent, request] = importBackup.mock.calls[0];
		expect(await (sent as Blob).text()).toBe('zip');
		expect(request).toEqual({ sections: ['machine'], devices: {} });
		await expect.element(page.getByText('3. Résultat')).toBeVisible();
		await expect.element(page.getByText(/Enregistrée, mais pas appliquée/)).toBeVisible();
		await expect.element(page.getByText('no home')).toBeVisible();
		expect(onchange).toHaveBeenCalled();
	});

	it('gives the configuration of a device that is away to a connected deck', async () => {
		const importBackup = vi.fn().mockResolvedValue({ ok: true, value: REPORT });
		const { input } = await renderImport({ importBackup });
		pick(input, ZIP);
		await page.getByRole('combobox', { name: /Appliquer à/ }).selectOptions('NEW');
		await expect.element(page.getByText('Sera appliquée à Stream Deck neuf.')).toBeVisible();
		await page.getByRole('button', { name: 'Importer 2 sections' }).click();
		expect(importBackup.mock.calls[0][1]).toEqual({
			sections: ['devices/OLD', 'machine'],
			devices: { OLD: 'NEW' }
		});
	});

	it('imports nothing when nothing is ticked', async () => {
		const importBackup = vi.fn();
		const { input } = await renderImport({ importBackup });
		pick(input, ZIP);
		await page.getByRole('checkbox', { name: /Stream Deck OLD/ }).click();
		await page.getByRole('checkbox', { name: 'Alias du shell' }).click();
		const button = page.getByRole('button', { name: 'Rien à importer' });
		await expect.element(button).toHaveAttribute('aria-disabled', 'true');
		await button.click({ force: true });
		expect(importBackup).not.toHaveBeenCalled();
	});

	it('undoes the import with its snapshot, after a confirmation', async () => {
		const restoreSnapshot = vi.fn().mockResolvedValue({ ok: true, value: REPORT });
		const { input, onchange } = await renderImport({ restoreSnapshot });
		pick(input, ZIP);
		await page.getByRole('button', { name: 'Importer 2 sections' }).click();
		await page.getByRole('button', { name: 'Annuler l’import' }).click();
		await expect.element(page.getByRole('dialog')).toBeVisible();
		await page.getByRole('button', { name: 'Annuler', exact: true }).click();
		expect(restoreSnapshot).not.toHaveBeenCalled();

		await page.getByRole('button', { name: 'Annuler l’import' }).click();
		await page.getByRole('dialog').getByRole('button', { name: 'Annuler l’import' }).click();
		expect(restoreSnapshot).toHaveBeenCalledWith('1-0');
		await expect.element(page.getByText('Import annulé', { exact: true })).toBeVisible();
		expect(onchange).toHaveBeenCalledTimes(2);
	});

	it('keeps the first step and says why when the file is refused', async () => {
		const previewBackup = vi.fn().mockResolvedValue({
			ok: false,
			problem: { text: 'Fichier refusé : ce n’est pas un export HuBergamof valide.', detail: 'bad' }
		});
		const { input } = await renderImport({ previewBackup });
		pick(input, ZIP);
		await expect.element(page.getByText(/Import non effectué/)).toBeVisible();
		await expect.element(page.getByText('1. Choisir le fichier')).toBeVisible();
	});

	it('shows a refusal of the import and stays on the preview', async () => {
		const importBackup = vi.fn().mockResolvedValue({
			ok: false,
			problem: { text: 'L’ordinateur est verrouillé.', detail: null }
		});
		const { input, onchange } = await renderImport({ importBackup });
		pick(input, ZIP);
		await page.getByRole('button', { name: 'Importer 2 sections' }).click();
		await expect.element(page.getByText(/Import non effectué/)).toBeVisible();
		await expect.element(page.getByText('2. Vérifier ce qui sera importé')).toBeVisible();
		expect(onchange).not.toHaveBeenCalled();
	});

	it('starts over from the report', async () => {
		const { input } = await renderImport({});
		pick(input, ZIP);
		await page.getByRole('button', { name: 'Importer 2 sections' }).click();
		await page.getByRole('button', { name: 'Importer un autre fichier' }).click();
		await expect.element(page.getByText('1. Choisir le fichier')).toBeVisible();
	});
});
