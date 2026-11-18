import { describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import ProblemAlert from './ProblemAlert.svelte';

describe('ProblemAlert', () => {
	it('announces the French text, with the lead before it', async () => {
		await render(ProblemAlert, {
			problem: { text: 'Le gestionnaire est injoignable.', detail: null },
			lead: 'Réglages non enregistrés.'
		});
		await expect
			.element(page.getByRole('alert'))
			.toHaveTextContent('Réglages non enregistrés. Le gestionnaire est injoignable.');
		await expect.element(page.getByText(/Détail/)).not.toBeInTheDocument();
	});

	it('shows the details of the manager in English', async () => {
		const { container } = await render(ProblemAlert, {
			problem: { text: 'Configuration refusée.', detail: 'invalid brightness' }
		});
		await expect.element(page.getByText(/Détail/)).toBeVisible();
		expect(container.querySelector('span[lang="en"]')?.textContent).toBe('invalid brightness');
	});
});
