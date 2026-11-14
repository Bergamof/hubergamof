import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import LockSleepControl from './LockSleepControl.svelte';

describe('LockSleepControl', () => {
	it('shows the delay and reports a new one', async () => {
		const onchange = vi.fn();
		await render(LockSleepControl, { seconds: 5, onchange });
		const field = page.getByLabelText('Extinction quand l’ordinateur est verrouillé');
		await expect.element(field).toHaveValue('5');
		await expect.element(page.getByText(/cadenas/)).toBeVisible();

		await field.fill('30');
		expect(onchange).toHaveBeenLastCalledWith(30);
	});

	it('refuses a delay beyond five minutes', async () => {
		const onchange = vi.fn();
		await render(LockSleepControl, { seconds: 5, onchange });
		await page.getByLabelText(/Extinction/).fill('301');
		expect(onchange).not.toHaveBeenCalled();
		await expect
			.element(page.getByText('Saisissez un nombre entier de secondes entre 0 et 300.'))
			.toBeVisible();
	});
});
