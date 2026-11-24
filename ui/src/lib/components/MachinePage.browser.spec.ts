import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import { emptyGit } from '#lib/deck/git-test-support.js';
import MachinePage from './MachinePage.svelte';

async function renderPage() {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	await render(MachinePage, { deck });
	return { connection };
}

describe('MachinePage', () => {
	it('opens on the shell aliases, and reads both topics from the computer', async () => {
		const { connection } = await renderPage();
		await expect.element(page.getByRole('radio', { name: 'Alias du shell' })).toBeChecked();
		expect(connection.sent).toContainEqual({ type: 'refreshMachine' });
		expect(connection.sent).toContainEqual({ type: 'refreshGit' });
	});

	it('shows Git when it is chosen', async () => {
		await renderPage();
		await page.getByRole('radio', { name: 'Git' }).click({ force: true });
		await expect
			.element(page.getByRole('heading', { name: 'Identité', exact: true }))
			.toBeVisible();
	});

	it('shows SSH when it is chosen, and reads the computer for it at the start', async () => {
		const { connection } = await renderPage();
		expect(connection.sent).toContainEqual({ type: 'refreshSsh' });
		await page.getByRole('radio', { name: 'SSH' }).click({ force: true });
		await expect.element(page.getByRole('heading', { name: 'Serveurs SSH' })).toBeVisible();
	});

	it('keeps what was typed in one topic while the other is shown', async () => {
		const { connection } = await renderPage();
		connection.receive({
			type: 'git',
			config: emptyGit(1),
			systemEntries: [],
			gitAvailable: true
		});
		await page.getByRole('radio', { name: 'Git' }).click({ force: true });
		const form = page.getByRole('form', { name: 'Identité', exact: true });
		await form.getByRole('textbox', { name: 'Nom' }).fill('Ada');
		await page.getByRole('radio', { name: 'Alias du shell' }).click({ force: true });
		await page.getByRole('radio', { name: 'Git' }).click({ force: true });
		await expect.element(form.getByRole('textbox', { name: 'Nom' })).toHaveValue('Ada');
	});

	it('goes back to the aliases', async () => {
		await renderPage();
		await page.getByRole('radio', { name: 'Git' }).click({ force: true });
		await page.getByRole('radio', { name: 'Alias du shell' }).click({ force: true });
		await expect.element(page.getByRole('heading', { name: 'Alias du shell' })).toBeVisible();
	});
});
