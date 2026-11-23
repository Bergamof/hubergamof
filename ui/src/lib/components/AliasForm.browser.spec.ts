import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { EditOutcome } from '#lib/deck/edit-queue.svelte.js';
import AliasForm from './AliasForm.svelte';

const SAVED: EditOutcome = { saved: true };

async function renderForm(props: Partial<Parameters<typeof render<typeof AliasForm>>[1]> = {}) {
	const onsubmit = vi.fn(async () => SAVED);
	await render(AliasForm, {
		takenNames: ['gs'],
		submitLabel: 'Ajouter',
		onsubmit,
		...props
	});
	return { onsubmit };
}

describe('AliasForm', () => {
	it('sends the alias, on by default, and empties itself once it was saved', async () => {
		const { onsubmit } = await renderForm();
		await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('ll');
		await page.getByRole('textbox', { name: 'Commande', exact: true }).fill('  ls -l  ');
		await page.getByRole('button', { name: 'Ajouter' }).click();
		expect(onsubmit).toHaveBeenCalledWith({ name: 'll', command: 'ls -l', enabled: true });
		await expect.element(page.getByRole('textbox', { name: 'Nom', exact: true })).toHaveValue('');
		await expect
			.element(page.getByRole('textbox', { name: 'Commande', exact: true }))
			.toHaveValue('');
		await expect.element(page.getByRole('textbox', { name: 'Nom', exact: true })).toHaveFocus();
	});

	it('can add an alias switched off', async () => {
		const { onsubmit } = await renderForm();
		await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('ll');
		await page.getByRole('textbox', { name: 'Commande', exact: true }).fill('ls');
		await page.getByRole('switch', { name: 'Activé' }).click();
		await page.getByRole('button', { name: 'Ajouter' }).click();
		expect(onsubmit).toHaveBeenCalledWith({ name: 'll', command: 'ls', enabled: false });
	});

	it('says what is wrong under each field, tied to it, and focuses the first one', async () => {
		const { onsubmit } = await renderForm();
		await page.getByRole('button', { name: 'Ajouter' }).click();
		expect(onsubmit).not.toHaveBeenCalled();
		const name = page.getByRole('textbox', { name: 'Nom', exact: true });
		await expect.element(name).toHaveAttribute('aria-invalid', 'true');
		await expect.element(name).toHaveAccessibleDescription('Saisissez le nom de l’alias.');
		await expect
			.element(page.getByRole('textbox', { name: 'Commande', exact: true }))
			.toHaveAccessibleDescription('Saisissez la commande.');
		await expect.element(name).toHaveFocus();
	});

	it('focuses the command when only it is wrong, and follows what is typed afterwards', async () => {
		await renderForm();
		await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('ll');
		await page.getByRole('button', { name: 'Ajouter' }).click();
		await expect
			.element(page.getByRole('textbox', { name: 'Commande', exact: true }))
			.toHaveFocus();
		await page.getByRole('textbox', { name: 'Commande', exact: true }).fill('ls');
		await expect
			.element(page.getByRole('textbox', { name: 'Commande', exact: true }))
			.toHaveAttribute('aria-invalid', 'false');
		expect(document.body.textContent).not.toContain('Saisissez la commande.');
	});

	it('refuses a name that is taken or malformed', async () => {
		await renderForm();
		await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('gs');
		await page.getByRole('textbox', { name: 'Commande', exact: true }).fill('git status');
		await page.getByRole('button', { name: 'Ajouter' }).click();
		await expect.element(page.getByText('Un alias porte déjà ce nom.')).toBeVisible();
		await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('1x');
		await expect.element(page.getByText(/commence par une lettre/)).toBeVisible();
	});

	it('keeps what was typed when the alias was not saved', async () => {
		const onsubmit = vi.fn(async (): Promise<EditOutcome> => ({
			saved: false,
			problem: { text: 'non', detail: null }
		}));
		await renderForm({ onsubmit });
		await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('ll');
		await page.getByRole('textbox', { name: 'Commande', exact: true }).fill('ls');
		await page.getByRole('button', { name: 'Ajouter' }).click();
		await expect.element(page.getByRole('textbox', { name: 'Nom', exact: true })).toHaveValue('ll');
	});

	it('starts from the alias being changed and can be given up', async () => {
		const oncancel = vi.fn();
		const ondone = vi.fn();
		const { onsubmit } = await renderForm({
			initial: { name: 'gs', command: 'git status', enabled: false },
			takenNames: [],
			submitLabel: 'Enregistrer',
			oncancel,
			ondone
		});
		await expect.element(page.getByRole('textbox', { name: 'Nom', exact: true })).toHaveValue('gs');
		await page.getByRole('textbox', { name: 'Commande', exact: true }).fill('git status -s');
		await page.getByRole('button', { name: 'Enregistrer' }).click();
		expect(onsubmit).toHaveBeenCalledWith({ name: 'gs', command: 'git status -s', enabled: false });
		expect(ondone).toHaveBeenCalledOnce();
		await page.getByRole('button', { name: 'Annuler' }).click();
		expect(oncancel).toHaveBeenCalledOnce();
	});
});
