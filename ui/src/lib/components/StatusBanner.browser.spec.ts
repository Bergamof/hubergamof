import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import StatusBanner from './StatusBanner.svelte';

describe('StatusBanner', () => {
	it('says nothing while the page can be used', async () => {
		await render(StatusBanner, { locked: false, online: true });
		await expect.element(page.getByText(/verrouillé/)).not.toBeInTheDocument();
		await expect
			.element(page.getByText(/Connexion au gestionnaire perdue/))
			.not.toBeInTheDocument();
	});

	it('says the computer is locked', async () => {
		await render(StatusBanner, { locked: true, online: true });
		await expect.element(page.getByText(/Ordinateur verrouillé\./)).toBeVisible();
		await expect.element(page.getByText(/lecture seule/)).toBeVisible();
	});

	it('says the connection is lost', async () => {
		await render(StatusBanner, { locked: false, online: false });
		await expect.element(page.getByText(/Connexion au gestionnaire perdue\./)).toBeVisible();
	});

	it('puts the lock before the lost connection', async () => {
		await render(StatusBanner, { locked: true, online: false });
		await expect.element(page.getByText(/Ordinateur verrouillé\./)).toBeVisible();
		await expect
			.element(page.getByText(/Connexion au gestionnaire perdue/))
			.not.toBeInTheDocument();
	});
});
