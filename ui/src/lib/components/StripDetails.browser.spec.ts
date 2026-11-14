import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { StripConfig } from '#lib/generated/index.js';
import StripDetails from './StripDetails.svelte';

const STRIP: StripConfig = {
	showSeconds: true,
	showPomodoroProgress: true,
	systemRefreshSeconds: 2,
	showHistory: true,
	loadAlertPercent: 85,
	lockSleepSeconds: 5
};

async function show(strip: Partial<StripConfig> = {}) {
	const onchange = vi.fn();
	await render(StripDetails, { strip: { ...STRIP, ...strip }, headingId: 'heading', onchange });
	return { onchange };
}

describe('StripDetails', () => {
	it('shows the options of the strip as they are', async () => {
		await show({ showSeconds: false, systemRefreshSeconds: 5 });
		await expect.element(page.getByRole('heading', { name: 'Bande tactile' })).toBeVisible();
		await expect.element(page.getByLabelText('Afficher les secondes')).not.toBeChecked();
		await expect.element(page.getByLabelText(/barre de progression du pomodoro/)).toBeChecked();
		await expect.element(page.getByLabelText(/historique des mesures/)).toBeChecked();
		await expect.element(page.getByLabelText(/Actualisation des mesures/)).toHaveValue('5');
		await expect
			.element(page.getByLabelText('Alerte de charge (en pourcentage)'))
			.toHaveValue('85');
	});

	it('reports each change at once', async () => {
		const { onchange } = await show();
		await page.getByLabelText('Afficher les secondes').click();
		expect(onchange).toHaveBeenLastCalledWith({ showSeconds: false });
		await page.getByLabelText(/barre de progression du pomodoro/).click();
		expect(onchange).toHaveBeenLastCalledWith({ showPomodoroProgress: false });
		await page.getByLabelText(/historique des mesures/).click();
		expect(onchange).toHaveBeenLastCalledWith({ showHistory: false });
		await page.getByLabelText(/Actualisation des mesures/).selectOptions('Toutes les 10 s');
		expect(onchange).toHaveBeenLastCalledWith({ systemRefreshSeconds: 10 });
	});

	it('sends the alert load once it is a valid percentage', async () => {
		const { onchange } = await show();
		const field = page.getByLabelText('Alerte de charge (en pourcentage)');
		await field.fill('90');
		expect(onchange).toHaveBeenLastCalledWith({ loadAlertPercent: 90 });
		onchange.mockClear();
		await field.fill('20');
		await expect
			.element(page.getByText('Saisissez un nombre entier entre 50 et 100.'))
			.toBeVisible();
		await expect.element(field).toHaveAttribute('aria-invalid', 'true');
		await field.fill('abc');
		await field.fill('');
		expect(onchange).not.toHaveBeenCalled();
	});

	it('sends the lock delay once it is a valid number of seconds', async () => {
		const { onchange } = await show();
		const field = page.getByLabelText('Extinction au verrouillage de l’ordinateur');
		await expect.element(field).toHaveValue('5');
		await field.fill('30');
		expect(onchange).toHaveBeenLastCalledWith({ lockSleepSeconds: 30 });
		await field.fill('0');
		expect(onchange).toHaveBeenLastCalledWith({ lockSleepSeconds: 0 });
		onchange.mockClear();
		await field.fill('301');
		await expect
			.element(page.getByText('Saisissez un nombre entier de secondes entre 0 et 300.'))
			.toBeVisible();
		await expect.element(field).toHaveAttribute('aria-invalid', 'true');
		await field.fill('1.5');
		expect(onchange).not.toHaveBeenCalled();
	});
});
