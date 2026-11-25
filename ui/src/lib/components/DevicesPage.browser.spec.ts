import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import type { MouseInfo } from '#lib/generated/index.js';
import DevicesPage from './DevicesPage.svelte';
import ManagerHarness from './ManagerHarness.svelte';

function manager() {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	return { connection, deck };
}

describe('DevicesPage', () => {
	it('says what to do when no device is connected', async () => {
		const { deck } = manager();
		await render(ManagerHarness, { deck, page: DevicesPage });
		await expect
			.element(page.getByRole('heading', { name: 'Aucun périphérique connecté' }))
			.toBeVisible();
		await expect.element(page.getByText(/Recherche en continu/)).toBeVisible();
	});

	it('lists the devices that are connected', async () => {
		const { connection, deck } = manager();
		connection.receive({
			type: 'mice',
			mice: [{ serial: 'M1', displayName: 'MX Master 4' } as MouseInfo]
		} as never);
		await render(ManagerHarness, { deck, page: DevicesPage });
		await expect.element(page.getByRole('link', { name: /MX Master 4/ })).toBeVisible();
		await expect.element(page.getByText(/^1 connecté$/)).toBeVisible();
	});
});
