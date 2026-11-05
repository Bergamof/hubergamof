import { expect, test } from '@playwright/test';

test.describe('page d’accueil sans gestionnaire', () => {
	test.beforeEach(async ({ page }) => {
		await page.goto('/');
	});

	test('affiche le titre HuBergamof', async ({ page }) => {
		await expect(page).toHaveTitle('HuBergamof');
		await expect(page.getByRole('heading', { level: 1, name: 'HuBergamof' })).toBeVisible();
	});

	test('indique que la connexion au gestionnaire est en cours', async ({ page }) => {
		await expect(page.getByRole('main')).toContainText('Connexion au gestionnaire…');
		await expect(page.getByRole('status')).toHaveText('Non connecté au gestionnaire');
	});
});
