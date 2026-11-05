import { createRawSnippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import ConfirmDialog from './ConfirmDialog.svelte';

const message = createRawSnippet(() => ({ render: () => '<p>Ses réglages seront perdus.</p>' }));

async function renderDialog(open = true) {
	const onconfirm = vi.fn();
	const oncancel = vi.fn();
	const view = await render(ConfirmDialog, {
		open,
		title: 'Supprimer la touche ?',
		confirmLabel: 'Supprimer',
		children: message,
		onconfirm,
		oncancel
	});
	return { ...view, onconfirm, oncancel };
}

describe('ConfirmDialog', () => {
	it('opens as a named modal with the safe choice focused', async () => {
		await renderDialog();
		const dialog = page.getByRole('dialog', { name: 'Supprimer la touche ?' });
		await expect.element(dialog).toBeVisible();
		await expect.element(dialog).toHaveAccessibleDescription('Ses réglages seront perdus.');
		await expect.element(page.getByRole('button', { name: 'Annuler' })).toHaveFocus();
	});

	it('stays closed until asked', async () => {
		await renderDialog(false);
		await expect.element(page.getByRole('dialog')).not.toBeInTheDocument();
	});

	it('confirms without cancelling when closed afterwards', async () => {
		const { onconfirm, oncancel, rerender } = await renderDialog();
		await page.getByRole('button', { name: 'Supprimer' }).click();
		expect(onconfirm).toHaveBeenCalledOnce();
		await rerender({ open: false });
		await expect.element(page.getByRole('dialog')).not.toBeInTheDocument();
		expect(oncancel).not.toHaveBeenCalled();
	});

	it('cancels with its button or Escape', async () => {
		const { onconfirm, oncancel } = await renderDialog();
		await page.getByRole('button', { name: 'Annuler' }).click();
		expect(oncancel).toHaveBeenCalledOnce();
		await userEvent.keyboard('{Escape}');
		await expect.element(page.getByRole('dialog')).not.toBeInTheDocument();
		expect(oncancel).toHaveBeenCalledTimes(2);
		expect(onconfirm).not.toHaveBeenCalled();
	});
});
