import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import MachinePage from './MachinePage.svelte';

async function renderPage() {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	await render(MachinePage, { deck });
	return { connection };
}

describe('MachinePage', () => {
	it('opens on the shell aliases', async () => {
		const { connection } = await renderPage();
		await expect.element(page.getByRole('radio', { name: 'Alias du shell' })).toBeChecked();
		expect(connection.sent).toEqual([{ type: 'refreshMachine' }]);
	});

	it('shows Git, and reads the computer for it, when it is chosen', async () => {
		const { connection } = await renderPage();
		await page.getByRole('radio', { name: 'Git' }).click({ force: true });
		await expect.element(page.getByRole('heading', { name: 'Identité' })).toBeVisible();
		expect(connection.sent).toContainEqual({ type: 'refreshGit' });
	});

	it('goes back to the aliases', async () => {
		await renderPage();
		await page.getByRole('radio', { name: 'Git' }).click({ force: true });
		await page.getByRole('radio', { name: 'Alias du shell' }).click({ force: true });
		await expect.element(page.getByRole('heading', { name: 'Alias du shell' })).toBeVisible();
	});
});
