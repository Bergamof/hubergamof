import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import AliasList from './AliasList.svelte';

const ALIASES = [
	{ name: 'gs', command: 'git status', enabled: true },
	{ name: 'll', command: 'ls -l', enabled: false }
];

const THREE = [...ALIASES, { name: 'la', command: 'ls -a', enabled: true }];

async function renderList(aliases = ALIASES) {
	const handlers = {
		onreplace: vi.fn(async () => ({ saved: true as const })),
		ontoggle: vi.fn(),
		onremove: vi.fn()
	};
	const fallbackFocus = vi.fn();
	const screen = await render(AliasList, { aliases, fallbackFocus, ...handlers });
	return { ...handlers, fallbackFocus, screen };
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
		await expect.element(page.getByRole('switch', { name: 'Alias gs' })).toBeChecked();
		await expect.element(page.getByRole('switch', { name: 'Alias ll' })).not.toBeChecked();
	});

	it('reports a switch', async () => {
		const { ontoggle } = await renderList();
		await page.getByRole('switch', { name: 'Alias ll' }).click();
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

		await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('gs');
		await page.getByRole('textbox', { name: 'Commande', exact: true }).fill('git status -s');
		await page.getByRole('button', { name: 'Enregistrer' }).click();
		expect(onreplace).toHaveBeenCalledWith('gs', {
			name: 'gs',
			command: 'git status -s',
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

	it('puts the focus on the name field when a change is opened', async () => {
		await renderList();
		await page.getByRole('button', { name: 'Modifier l’alias ll' }).click();
		await expect.element(page.getByRole('textbox', { name: 'Nom', exact: true })).toHaveFocus();
	});

	it('gives the focus back to « Modifier » when a change is given up', async () => {
		await renderList();
		await page.getByRole('button', { name: 'Modifier l’alias ll' }).click();
		await page.getByRole('button', { name: 'Annuler' }).click();
		await expect.element(page.getByRole('button', { name: 'Modifier l’alias ll' })).toHaveFocus();
	});

	it('gives the focus back to « Modifier » of a renamed alias, once the list has its new name', async () => {
		const { screen, onreplace } = await renderList();
		await page.getByRole('button', { name: 'Modifier l’alias gs' }).click();
		await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('gst');
		await page.getByRole('button', { name: 'Enregistrer' }).click();
		expect(onreplace).toHaveBeenCalledOnce();
		// The manager answers a little later with the renamed alias.
		await screen.rerender({
			aliases: [{ name: 'gst', command: 'git status', enabled: true }, ALIASES[1]]
		});
		await expect.element(page.getByRole('button', { name: 'Modifier l’alias gst' })).toHaveFocus();
		expect(document.activeElement?.closest('li')?.dataset.alias).toBe('gst');
	});

	it('moves the focus to « Supprimer » of the next alias after a removal, else the previous one', async () => {
		const { screen } = await renderList(THREE);
		const dialog = page.getByRole('dialog', { name: 'Supprimer l’alias ?' });
		await page.getByRole('button', { name: 'Supprimer l’alias gs' }).click();
		await dialog.getByRole('button', { name: 'Supprimer' }).click();
		await expect.element(page.getByRole('button', { name: 'Supprimer l’alias ll' })).toHaveFocus();

		await screen.rerender({ aliases: THREE.slice(1) });
		await page.getByRole('button', { name: 'Supprimer l’alias la' }).click();
		await dialog.getByRole('button', { name: 'Supprimer' }).click();
		await expect.element(page.getByRole('button', { name: 'Supprimer l’alias ll' })).toHaveFocus();
	});

	it('hands the focus over when the last alias is removed', async () => {
		const { fallbackFocus } = await renderList(ALIASES.slice(0, 1));
		await page.getByRole('button', { name: 'Supprimer l’alias gs' }).click();
		await page
			.getByRole('dialog', { name: 'Supprimer l’alias ?' })
			.getByRole('button', { name: 'Supprimer' })
			.click();
		await vi.waitFor(() => expect(fallbackFocus).toHaveBeenCalledOnce());
	});

	it('leaves the focus on the opener when a removal is given up', async () => {
		const { fallbackFocus } = await renderList(THREE);
		await page.getByRole('button', { name: 'Supprimer l’alias gs' }).click();
		await page
			.getByRole('dialog', { name: 'Supprimer l’alias ?' })
			.getByRole('button', { name: 'Annuler' })
			.click();
		await expect.element(page.getByRole('button', { name: 'Supprimer l’alias gs' })).toHaveFocus();
		expect(fallbackFocus).not.toHaveBeenCalled();
	});
});
