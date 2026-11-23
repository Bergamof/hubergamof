import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import SystemAliasList from './SystemAliasList.svelte';

const ALIASES = [
	{ name: 'gs', command: 'git status', source: '.bashrc' },
	{ name: 'bad name', command: 'ls', source: '.zshrc' }
];

describe('SystemAliasList', () => {
	it('says when there is nothing else on the system', async () => {
		await render(SystemAliasList, { aliases: [], full: false, onadopt: vi.fn() });
		await expect.element(page.getByText('Aucun autre alias sur le système.')).toBeVisible();
	});

	it('names the file of each alias and says it is not managed', async () => {
		await render(SystemAliasList, { aliases: ALIASES, full: false, onadopt: vi.fn() });
		await expect.element(page.getByText('Non géré · lu dans ~/.bashrc')).toBeVisible();
		await expect.element(page.getByText('Non géré · lu dans ~/.zshrc')).toBeVisible();
	});

	it('takes an alias in', async () => {
		const onadopt = vi.fn();
		await render(SystemAliasList, { aliases: ALIASES, full: false, onadopt });
		await page.getByRole('button', { name: 'Intégrer l’alias gs' }).click();
		expect(onadopt).toHaveBeenCalledWith(ALIASES[0]);
	});

	it('explains why an alias cannot be taken in, and does not take it', async () => {
		const onadopt = vi.fn();
		await render(SystemAliasList, { aliases: ALIASES, full: false, onadopt });
		const button = page.getByRole('button', { name: 'Intégrer l’alias bad name' });
		await expect.element(button).toHaveAttribute('aria-disabled', 'true');
		await expect.element(button).toHaveAccessibleDescription(/ne respecte pas les règles/);
		await button.click({ force: true });
		expect(onadopt).not.toHaveBeenCalled();
	});

	it('takes nothing in when the configuration is full', async () => {
		await render(SystemAliasList, { aliases: ALIASES.slice(0, 1), full: true, onadopt: vi.fn() });
		await expect
			.element(page.getByRole('button', { name: 'Intégrer l’alias gs' }))
			.toHaveAccessibleDescription('La configuration est pleine.');
	});
});
