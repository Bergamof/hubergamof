import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import EmptyState from './EmptyState.svelte';

describe('EmptyState', () => {
	it('names what is missing', async () => {
		await render(EmptyState, { title: 'Aucun module dans l’application' });
		await expect
			.element(page.getByRole('heading', { name: 'Aucun module dans l’application', level: 2 }))
			.toBeVisible();
	});

	it('is announced only when the page asks for it', async () => {
		await render(EmptyState, { title: 'Ce Stream Deck n’est plus connecté', live: true });
		await expect.element(page.getByRole('status')).toBeVisible();
		await expect.element(page.getByText(/plus connecté/)).toBeVisible();
	});

	it('is not a live region by default', async () => {
		await render(EmptyState, { title: 'Aucun module' });
		await expect.element(page.getByRole('status')).not.toBeInTheDocument();
	});
});
