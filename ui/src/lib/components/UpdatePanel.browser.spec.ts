import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { UpdateSettings, UpdateStatus } from '#lib/generated/index.js';
import UpdatePanel from './UpdatePanel.svelte';

const AVAILABLE: UpdateStatus = {
	type: 'available',
	version: '1.2.0',
	notesUrl: 'https://example.org/notes'
};
const CAN_INSTALL: UpdateSettings = { autoInstall: false, installSupport: 'available' };
const CANNOT_INSTALL: UpdateSettings = { autoInstall: true, installSupport: 'unpackaged' };

function props(overrides: Partial<Parameters<typeof render<typeof UpdatePanel>>[1]> = {}) {
	return {
		version: '1.0.0',
		status: { type: 'upToDate' } as UpdateStatus,
		settings: CAN_INSTALL,
		oncheck: vi.fn(),
		oninstall: vi.fn(),
		onautoinstall: vi.fn(),
		...overrides
	};
}

const checkButton = () => page.getByRole('button', { name: 'Rechercher des mises à jour' });

describe('UpdatePanel', () => {
	it('announces a newer release with a safe link to its notes', async () => {
		await render(UpdatePanel, props({ status: AVAILABLE }));
		await expect.element(page.getByText('Version 1.2.0 disponible.')).toBeVisible();
		await expect.element(page.getByText(/Vous utilisez la version 1\.0\.0/)).toBeVisible();
		const link = page.getByRole('link', { name: /Notes de version/ });
		await expect.element(link).toHaveAttribute('href', 'https://example.org/notes');
		await expect.element(link).toHaveAttribute('target', '_blank');
		await expect.element(link).toHaveAttribute('rel', 'noopener noreferrer');
	});

	it('offers no link for anything but https', async () => {
		await render(
			UpdatePanel,
			props({ status: { ...AVAILABLE, notesUrl: 'javascript:alert(1)' } as UpdateStatus })
		);
		await expect.element(page.getByText('Version 1.2.0 disponible.')).toBeVisible();
		expect(document.querySelectorAll('a')).toHaveLength(0);
	});

	it.each([
		['upToDate', {}, /HuBergamof est à jour \(version 1\.0\.0\)/],
		['checking', {}, /Recherche de mises à jour…/],
		['unknown', {}, /Version 1\.0\.0\. Aucune recherche/],
		['downloading', { version: '1.2.0' }, /Téléchargement de la version 1\.2\.0…/],
		['installing', { version: '1.2.0' }, /Installation de la version 1\.2\.0…/],
		['installed', { version: '1.2.0' }, /Version 1\.2\.0 installée : le service redémarre/]
	])('shows the %s state', async (type, extra, text) => {
		await render(UpdatePanel, props({ status: { type, ...extra } as UpdateStatus }));
		await expect.element(page.getByText(text)).toBeVisible();
	});

	it('shows the detail of a failure as English text', async () => {
		await render(UpdatePanel, props({ status: { type: 'checkFailed', message: 'offline' } }));
		await expect.element(page.getByText(/La recherche de mises à jour a échoué\./)).toBeVisible();
		await expect.element(page.getByText('offline')).toHaveAttribute('lang', 'en');
	});

	it('copes with an unknown running version', async () => {
		await render(UpdatePanel, props({ version: undefined }));
		await expect.element(page.getByText('HuBergamof est à jour.')).toBeVisible();
	});

	it('asks for a check, except while something runs', async () => {
		const oncheck = vi.fn();
		const view = await render(UpdatePanel, props({ oncheck }));
		await checkButton().click();
		expect(oncheck).toHaveBeenCalledTimes(1);

		await view.rerender(props({ oncheck, status: { type: 'checking' } }));
		await expect.element(checkButton()).toHaveAttribute('aria-disabled', 'true');
		await checkButton().click({ force: true });
		expect(oncheck).toHaveBeenCalledTimes(1);
	});

	it('installs a newer release on request', async () => {
		const oninstall = vi.fn();
		await render(UpdatePanel, props({ status: AVAILABLE, oninstall }));
		await page.getByRole('button', { name: 'Mettre à jour maintenant' }).click();
		expect(oninstall).toHaveBeenCalledTimes(1);
	});

	it('does not install twice while downloading', async () => {
		const oninstall = vi.fn();
		const view = await render(UpdatePanel, props({ status: AVAILABLE, oninstall }));
		await view.rerender(props({ status: { type: 'downloading', version: '1.2.0' }, oninstall }));
		// The button is gone while downloading: nothing can start a second installation.
		expect(document.querySelectorAll('button')).toHaveLength(1);
		expect(oninstall).not.toHaveBeenCalled();
	});

	it('offers to retry a failed installation, with its detail', async () => {
		const oninstall = vi.fn();
		await render(
			UpdatePanel,
			props({
				status: { type: 'installFailed', version: '1.2.0', message: 'signature mismatch' },
				oninstall
			})
		);
		await expect
			.element(page.getByText(/L’installation de la version 1\.2\.0 a échoué\./))
			.toBeVisible();
		await expect.element(page.getByText('signature mismatch')).toHaveAttribute('lang', 'en');
		await page.getByRole('button', { name: 'Réessayer' }).click();
		expect(oninstall).toHaveBeenCalledTimes(1);
	});

	it('explains that an installation outside a package updates by hand', async () => {
		await render(UpdatePanel, props({ status: AVAILABLE, settings: CANNOT_INSTALL }));
		await expect.element(page.getByText(/ne se met pas à jour seule/)).toBeVisible();
		expect(document.querySelectorAll('input')).toHaveLength(0);
		await expect
			.element(page.getByRole('button', { name: 'Mettre à jour maintenant' }))
			.not.toBeInTheDocument();
	});

	it('shows no install control before the manager has said what it supports', async () => {
		await render(UpdatePanel, props({ status: AVAILABLE, settings: undefined }));
		expect(document.querySelectorAll('input')).toHaveLength(0);
		await expect
			.element(page.getByRole('button', { name: 'Mettre à jour maintenant' }))
			.not.toBeInTheDocument();
	});

	it('shows and changes the automatic installation', async () => {
		const onautoinstall = vi.fn();
		await render(UpdatePanel, props({ onautoinstall }));
		const toggle = page.getByRole('checkbox', {
			name: 'Installer automatiquement les mises à jour'
		});
		await expect.element(toggle).not.toBeChecked();
		await toggle.click();
		expect(onautoinstall).toHaveBeenLastCalledWith(true);
	});
});
