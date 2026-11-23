import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import AliasList from './AliasList.svelte';

const ALIASES = [
	{ name: 'gs', command: 'git status', enabled: true },
	{ name: 'll', command: 'ls -l', enabled: false }
];

async function renderList(aliases = ALIASES) {
	const handlers = {
		onreplace: vi.fn(async () => ({ saved: true as const })),
		ontoggle: vi.fn(),
		onremove: vi.fn()
	};
	await render(AliasList, { aliases, ...handlers });
	return handlers;
}

describe('AliasList', () => {
	it('says when there is no alias', async () => {
		await renderList([]);
		await expect.element(page.getByText('Aucun alias géré pour l’instant.')).toBeVisible();
	});

	it('shows each alias with its command, and marks the ones that are off in words', async () => {
		await renderList();
		const list = page.getByRole('list', { name: 'Alias gérés' });
		await expect.element(list.getByText('git status')).toBeVisible();
		await expect.element(list.getByText('Désactivé')).toBeVisible();
		await expect.element(page.getByRole('switch', { name: 'Activer l’alias gs' })).toBeChecked();
		await expect
			.element(page.getByRole('switch', { name: 'Activer l’alias ll' }))
			.not.toBeChecked();
	});

	it('reports a switch', async () => {
		const { ontoggle } = await renderList();
		await page.getByRole('switch', { name: 'Activer l’alias ll' }).click();
		expect(ontoggle).toHaveBeenCalledWith('ll', true);
	});

	it('removes an alias only once confirmed', async () => {
		const { onremove } = await renderList();
		await page.getByRole('button', { name: 'Supprimer l’alias gs' }).click();
		const dialog = page.getByRole('dialog', { name: 'Supprimer l’alias ?' });
		await expect.element(dialog).toBeVisible();
		await dialog.getByRole('button', { name: 'Annuler' }).click();
		expect(onremove).not.toHaveBeenCalled();

		await page.getByRole('button', { name: 'Supprimer l’alias gs' }).click();
		await dialog.getByRole('button', { name: 'Supprimer' }).click();
		expect(onremove).toHaveBeenCalledWith('gs');
	});

	it('changes an alias in place, refusing the name of another one, and gives the focus back', async () => {
		const { onreplace } = await renderList();
		await page.getByRole('button', { name: 'Modifier l’alias gs' }).click();
		await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('ll');
		await page.getByRole('button', { name: 'Enregistrer' }).click();
		await expect.element(page.getByText('Un alias porte déjà ce nom.')).toBeVisible();
		expect(onreplace).not.toHaveBeenCalled();

		await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('gst');
		await page.getByRole('button', { name: 'Enregistrer' }).click();
		expect(onreplace).toHaveBeenCalledWith('gs', {
			name: 'gst',
			command: 'git status',
			enabled: true
		});
		await expect.element(page.getByRole('button', { name: 'Modifier l’alias gs' })).toHaveFocus();
	});

	it('gives up a change', async () => {
		const { onreplace } = await renderList();
		await page.getByRole('button', { name: 'Modifier l’alias gs' }).click();
		await page.getByRole('button', { name: 'Annuler' }).click();
		expect(onreplace).not.toHaveBeenCalled();
		await expect.element(page.getByRole('button', { name: 'Modifier l’alias gs' })).toBeVisible();
	});
});
