import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { ALIASES_MAX } from '#lib/deck/alias-validation.js';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import type { MachineConfig, SystemAlias } from '#lib/generated/index.js';
import ShellAliasesPanel from './ShellAliasesPanel.svelte';

const SYSTEM: SystemAlias[] = [{ name: 'gs', command: 'git status', source: '.bashrc' }];

function machine(revision: number, names: string[] = ['ll']): MachineConfig {
	return {
		schemaVersion: 1,
		aliases: names.map((name) => ({ name, command: 'ls -l', enabled: true })),
		revision
	};
}

async function renderPage(config: MachineConfig | null = machine(1)) {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	await render(ShellAliasesPanel, { deck });
	if (config) connection.receive({ type: 'machine', config, systemAliases: SYSTEM });
	return { connection };
}

describe('ShellAliasesPanel', () => {
	it('asks the manager to read the system when it opens', async () => {
		const { connection } = await renderPage();
		expect(connection.sent[0]).toEqual({ type: 'refreshMachine' });
	});

	it('waits for the configuration', async () => {
		await renderPage(null);
		await expect.element(page.getByText('Chargement des alias…')).toBeVisible();
	});

	it('lists the managed aliases and, apart, those of the system with their limit', async () => {
		await renderPage();
		await expect.element(page.getByRole('heading', { name: 'Alias du shell' })).toBeVisible();
		await expect
			.element(page.getByRole('list', { name: 'Alias gérés' }).getByText('ll', { exact: true }))
			.toBeVisible();
		const system = page.getByRole('list', { name: 'Alias du système, non gérés' });
		await expect.element(system.getByText('git status')).toBeVisible();
		await expect.element(page.getByText(/Seuls ~\/\.bashrc/)).toBeVisible();
	});

	it('sends the whole configuration on its revision when an alias is added', async () => {
		const { connection } = await renderPage();
		await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('la');
		await page.getByRole('textbox', { name: 'Commande', exact: true }).fill('ls -a');
		await page.getByRole('button', { name: 'Ajouter' }).click();
		expect(connection.lastSent()).toMatchObject({
			type: 'updateMachine',
			config: {
				revision: 1,
				aliases: [
					{ name: 'll', command: 'ls -l', enabled: true },
					{ name: 'la', command: 'ls -a', enabled: true }
				]
			}
		});
		await expect.element(page.getByText('Enregistrement…').first()).toBeVisible();

		connection.receive({
			type: 'machine',
			config: {
				...machine(2, ['ll', 'la']),
				aliases: [
					{ name: 'll', command: 'ls -l', enabled: true },
					{ name: 'la', command: 'ls -a', enabled: true }
				]
			},
			systemAliases: SYSTEM
		});
		await expect.element(page.getByText('Modifications enregistrées.').first()).toBeVisible();
		await expect.element(page.getByRole('textbox', { name: 'Nom', exact: true })).toHaveValue('');
	});

	it('sends a switch and a removal', async () => {
		const { connection } = await renderPage();
		await page.getByRole('switch', { name: 'Alias ll' }).click();
		expect(connection.lastSent()).toMatchObject({
			type: 'updateMachine',
			config: { aliases: [{ name: 'll', enabled: false }] }
		});
	});

	it('takes a system alias in, switched on, and removes it from the system list at once', async () => {
		const { connection } = await renderPage();
		await page.getByRole('button', { name: 'Intégrer l’alias gs' }).click();
		expect(connection.lastSent()).toMatchObject({
			type: 'updateMachine',
			config: {
				aliases: [
					{ name: 'll', enabled: true },
					{ name: 'gs', command: 'git status', enabled: true }
				]
			}
		});
		await expect.element(page.getByText('Aucun autre alias sur le système.')).toBeVisible();
	});

	it('does not offer to add an alias once the configuration is full', async () => {
		await renderPage(
			machine(
				1,
				Array.from({ length: ALIASES_MAX }, (_, index) => `a${index}`)
			)
		);
		await expect.element(page.getByText(/nombre maximal d’alias/)).toBeVisible();
		expect(document.querySelector('form')).toBeNull();
	});

	it('focuses the section title when the last system alias is taken in', async () => {
		await renderPage();
		await page.getByRole('button', { name: 'Intégrer l’alias gs' }).click();
		await expect
			.element(page.getByRole('heading', { name: 'Alias présents sur le système, non gérés' }))
			.toHaveFocus();
	});

	it('focuses the name of the form to add one when the last managed alias is removed', async () => {
		await renderPage();
		await page.getByRole('button', { name: 'Supprimer l’alias ll' }).click();
		await page
			.getByRole('dialog', { name: 'Supprimer l’alias ?' })
			.getByRole('button', { name: 'Supprimer' })
			.click();
		await expect.element(page.getByRole('textbox', { name: 'Nom', exact: true })).toHaveFocus();
	});

	it('names the form to add an alias by its title', async () => {
		await renderPage();
		await expect.element(page.getByRole('form', { name: 'Ajouter un alias' })).toBeVisible();
	});
});
