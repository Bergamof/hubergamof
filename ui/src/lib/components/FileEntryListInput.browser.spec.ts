import { describe, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ApiResult, ManagerApi } from '#lib/api/manager-api.js';
import type { FileEntryDraft } from '#lib/deck/settings-form.js';
import FileEntryListInput from './FileEntryListInput.svelte';

const SHOP: FileEntryDraft = { path: '/srv/shop/compose.yaml', name: 'Boutique', icon: 'shop.png' };
const BLOG: FileEntryDraft = { path: '~/blog/compose.yaml', name: '', icon: null };

function fakeApi(chosen: ApiResult<string | null> = { ok: true, value: null }) {
	const api = {
		chooseComposeFile: vi.fn().mockResolvedValue(chosen),
		uploadKeyImage: vi.fn().mockResolvedValue({ ok: true, value: 'uploaded.png' })
	};
	return { api, asApi: api as unknown as ManagerApi };
}

async function show(entries: FileEntryDraft[], chosen?: ApiResult<string | null>) {
	const { api, asApi } = fakeApi(chosen);
	const onchange = vi.fn();
	const onediting = vi.fn();
	await render(FileEntryListInput, {
		id: 'files',
		labelledBy: 'files-label',
		describedBy: undefined,
		entries,
		serial: 'DECK',
		api: asApi,
		defaultIcon: 'data:image/png;base64,AAAA',
		invalid: false,
		onediting,
		onchange
	});
	return { api, onchange, onediting };
}

describe('FileEntryListInput', () => {
	it('says when no file was added yet', async () => {
		await show([]);
		await expect.element(page.getByText('Aucun fichier ajouté.')).toBeVisible();
		await expect.element(page.getByRole('button', { name: 'Ajouter un fichier…' })).toBeVisible();
	});

	it('lists the files with their name, path and icon, the folder naming a nameless one', async () => {
		await show([SHOP, BLOG]);
		await expect.element(page.getByText('Boutique')).toBeVisible();
		await expect.element(page.getByText('/srv/shop/compose.yaml')).toBeVisible();
		await expect.element(page.getByText(/^blog/)).toBeVisible();
		await expect.element(page.getByText('(nom par défaut)')).toBeVisible();
		// Decorative images (empty alternative text, hidden from assistive technologies).
		const sources = [...document.querySelectorAll('li img')].map((icon) =>
			icon.getAttribute('src')
		);
		expect(sources).toEqual(['/api/devices/DECK/images/shop.png', 'data:image/png;base64,AAAA']);
	});

	it('adds the file chosen in the desktop dialog, with a name and an icon', async () => {
		const { api, onchange } = await show([], { ok: true, value: '/srv/api/compose.yaml' });
		await page.getByRole('button', { name: 'Ajouter un fichier…' }).click();
		expect(api.chooseComposeFile).toHaveBeenCalledOnce();
		await expect.element(page.getByLabelText('Fichier')).toHaveValue('/srv/api/compose.yaml');
		await page.getByLabelText('Nom').fill('API interne');
		const input = document.querySelector<HTMLInputElement>('input[type="file"]');
		await userEvent.upload(
			input as HTMLInputElement,
			new File(['x'], 'logo.png', { type: 'image/png' })
		);
		await expect.element(page.getByText('Icône choisie.')).toBeVisible();
		await page.getByRole('button', { name: 'Ajouter à la liste' }).click();
		expect(onchange).toHaveBeenCalledExactlyOnceWith([
			{ path: '/srv/api/compose.yaml', name: 'API interne', icon: 'uploaded.png' }
		]);
	});

	it('does nothing when the dialog is closed without a choice', async () => {
		const { onchange } = await show([], { ok: true, value: null });
		await page.getByRole('button', { name: 'Ajouter un fichier…' }).click();
		await expect.element(page.getByLabelText('Fichier')).not.toBeInTheDocument();
		expect(onchange).not.toHaveBeenCalled();
	});

	it('lets the path be typed where there is no dialog', async () => {
		const { onchange } = await show([], {
			ok: false,
			problem: { text: 'Le sélecteur de fichier n’est pas disponible.', detail: null }
		});
		await page.getByRole('button', { name: 'Ajouter un fichier…' }).click();
		await expect.element(page.getByText(/sélecteur de fichier n’est pas disponible/)).toBeVisible();
		await page.getByLabelText('Fichier').fill(' ~/app/compose.yaml ');
		await page.getByRole('button', { name: 'Ajouter à la liste' }).click();
		expect(onchange).toHaveBeenCalledExactlyOnceWith([
			{ path: '~/app/compose.yaml', name: '', icon: null }
		]);
	});

	it('refuses a relative path, an empty one and a file already listed', async () => {
		const { onchange } = await show([SHOP], { ok: true, value: '/elsewhere.yaml' });
		await page.getByRole('button', { name: 'Ajouter un fichier…' }).click();
		const path = page.getByLabelText('Fichier');
		const add = page.getByRole('button', { name: 'Ajouter à la liste' });
		await path.fill('compose.yaml');
		await add.click();
		await expect.element(page.getByText(/chemin absolu attendu/)).toBeVisible();
		await path.fill('');
		await add.click();
		await expect.element(page.getByText('Indiquer le chemin du fichier.')).toBeVisible();
		await path.fill('/srv/shop/compose.yaml');
		await add.click();
		await expect.element(page.getByText(/est déjà dans la liste/)).toBeVisible();
		expect(onchange).not.toHaveBeenCalled();
	});

	it('edits a file in place and can restore its default icon', async () => {
		const { onchange } = await show([SHOP, BLOG]);
		await page.getByRole('button', { name: 'Modifier Boutique' }).click();
		await expect.element(page.getByLabelText('Fichier')).toHaveValue('/srv/shop/compose.yaml');
		await page.getByLabelText('Nom').fill('Ma boutique');
		await page.getByRole('button', { name: 'Rétablir l’icône par défaut' }).click();
		await page.getByRole('button', { name: 'Valider' }).click();
		expect(onchange).toHaveBeenCalledExactlyOnceWith([
			{ path: '/srv/shop/compose.yaml', name: 'Ma boutique', icon: null },
			BLOG
		]);
	});

	it('cancels an edit without changing anything', async () => {
		const { onchange } = await show([SHOP]);
		await page.getByRole('button', { name: 'Modifier Boutique' }).click();
		await page.getByLabelText('Nom').fill('Autre');
		await page.getByRole('button', { name: 'Annuler' }).click();
		await expect.element(page.getByText('Boutique')).toBeVisible();
		expect(onchange).not.toHaveBeenCalled();
	});

	it('confirms the file with Enter instead of submitting the surrounding form', async () => {
		const { onchange } = await show([SHOP]);
		await page.getByRole('button', { name: 'Modifier Boutique' }).click();
		await page.getByLabelText('Nom').fill('Entrée');
		await userEvent.keyboard('{Enter}');
		expect(onchange).toHaveBeenCalledExactlyOnceWith([{ ...SHOP, name: 'Entrée' }]);
	});

	it('removes a file', async () => {
		const { onchange } = await show([SHOP, BLOG]);
		await page.getByRole('button', { name: 'Supprimer Boutique' }).click();
		expect(onchange).toHaveBeenCalledExactlyOnceWith([BLOG]);
		await expect.element(page.getByText(/« Boutique » retiré/)).toBeInTheDocument();
	});

	it('reports an icon that cannot be stored and keeps the previous one', async () => {
		const { api, onchange } = await show([SHOP]);
		api.uploadKeyImage.mockResolvedValue({
			ok: false,
			problem: { text: 'Image refusée.', detail: null }
		});
		await page.getByRole('button', { name: 'Modifier Boutique' }).click();
		const input = document.querySelector<HTMLInputElement>('input[type="file"]');
		await userEvent.upload(
			input as HTMLInputElement,
			new File(['x'], 'bad.png', { type: 'image/png' })
		);
		await expect.element(page.getByText('Image refusée.')).toBeVisible();
		await page.getByRole('button', { name: 'Valider' }).click();
		expect(onchange).toHaveBeenCalledExactlyOnceWith([SHOP]);
	});

	it('cancels the edit with Escape and gives the focus back to the file', async () => {
		const { onchange } = await show([SHOP]);
		await page.getByRole('button', { name: 'Modifier Boutique' }).click();
		await page.getByLabelText('Nom').fill('Autre');
		await userEvent.keyboard('{Escape}');
		await expect.element(page.getByLabelText('Nom')).not.toBeInTheDocument();
		await expect.element(page.getByRole('button', { name: 'Modifier Boutique' })).toHaveFocus();
		expect(onchange).not.toHaveBeenCalled();
	});

	it('leaves the other files alone while one is being edited', async () => {
		const { api, onchange } = await show([SHOP, BLOG]);
		await page.getByRole('button', { name: 'Modifier Boutique' }).click();
		// aria-disabled buttons stay in the page and ignore the click (a real click would wait).
		for (const name of ['Supprimer blog', 'Modifier blog', 'Ajouter un fichier…']) {
			const button = page.getByRole('button', { name }).element() as HTMLElement;
			expect(button.getAttribute('aria-disabled')).toBe('true');
			button.click();
		}
		expect(onchange).not.toHaveBeenCalled();
		expect(api.chooseComposeFile).not.toHaveBeenCalled();
		await expect.element(page.getByLabelText('Fichier')).toHaveValue('/srv/shop/compose.yaml');
	});

	it('tells the form when a file is being edited', async () => {
		const { onediting } = await show([SHOP]);
		await page.getByRole('button', { name: 'Modifier Boutique' }).click();
		await expect.poll(() => onediting.mock.lastCall?.[0]).toBe(true);
		await page.getByRole('button', { name: 'Annuler' }).click();
		await expect.poll(() => onediting.mock.lastCall?.[0]).toBe(false);
	});

	it('puts the focus on the next file after a removal, else on the button adding one', async () => {
		await show([SHOP, BLOG]);
		await page.getByRole('button', { name: 'Supprimer Boutique' }).click();
		await expect.element(page.getByRole('button', { name: 'Modifier Boutique' })).toHaveFocus();
	});

	it('refuses a name with control characters', async () => {
		const { onchange } = await show([SHOP]);
		await page.getByRole('button', { name: 'Modifier Boutique' }).click();
		await page.getByLabelText('Nom').fill('a\u202Eb');
		await page.getByRole('button', { name: 'Valider' }).click();
		await expect.element(page.getByText(/caractère de contrôle/)).toBeVisible();
		expect(onchange).not.toHaveBeenCalled();
	});
});
