import type { Locator, Page, WebSocketRoute } from '@playwright/test';
import { expect, test } from './demo-manager.js';

// Each test runs against its own `bergadeck --demo` with the default layout of a Stream Deck+:
// keys Audio, Pomodoro, Docker, IntelliJ, Claude (the last two not provided yet, until the
// step that brings them); dials Horloge, Audio, Pomodoro, Navigation.

function key(page: Page, name: string | RegExp): Locator {
	return page.getByRole('button', { name: typeof name === 'string' ? `Touche ${name}` : name });
}

function dial(page: Page, name: string): Locator {
	return page.getByRole('button', { name: `Molette ${name}` });
}

/** Checks the accessible name of each control, in order. */
async function expectNames(controls: Locator, names: readonly string[]) {
	await expect(controls).toHaveCount(names.length);
	for (const [index, name] of names.entries()) {
		await expect(controls.nth(index)).toHaveAccessibleName(name);
	}
}

async function open(page: Page) {
	await page.goto('/');
	await expect(page.getByRole('status').first()).toHaveText('Connecté au gestionnaire');
	await expect(key(page, '1 : Audio')).toBeVisible();
}

test('affiche les touches et les molettes de la configuration', async ({ page }) => {
	await open(page);
	await expect(page.getByRole('heading', { name: 'Stream Deck+' })).toBeVisible();
	await expectNames(page.getByRole('button', { name: /^Touche / }), [
		'Touche 1 : Audio',
		'Touche 2 : Pomodoro',
		'Touche 3 : Docker',
		'Touche 4 : IntelliJ indisponible',
		'Touche 5 : Claude indisponible'
	]);
	await expect(key(page, '1 : Audio').locator('img')).toHaveAttribute('src', /^data:image\/png/);
	await expectNames(page.getByRole('button', { name: /^Molette / }), [
		'Molette 1 : Horloge',
		'Molette 2 : Audio',
		'Molette 3 : Pomodoro',
		'Molette 4 : Navigation'
	]);
});

test('ajoute une touche depuis le catalogue', async ({ page }) => {
	await open(page);
	await page.getByRole('button', { name: 'Ajouter une touche' }).click();
	const picker = page.getByRole('dialog', { name: 'Ajouter une touche' });
	await expect(picker.getByRole('button', { name: /Horloge/ })).toHaveCount(0);
	await picker.getByRole('button', { name: /^Pomodoro/ }).click();

	await expect(picker).toBeHidden();
	await expect(key(page, '6 : Pomodoro')).toHaveAttribute('aria-current', 'true');
	await expect(page.getByRole('heading', { name: 'Touche 6 : Pomodoro' })).toBeVisible();
	await expect(page.getByRole('status').filter({ hasText: 'ajoutée' })).toHaveText(
		'Touche Pomodoro ajoutée en position 6.'
	);
});

test('déplace une touche au clavier sans perdre le focus', async ({ page }) => {
	await open(page);
	await key(page, '1 : Audio').click();
	const before = page.getByRole('button', { name: 'Déplacer avant' });
	const after = page.getByRole('button', { name: 'Déplacer après' });

	await before.focus();
	await page.keyboard.press('Enter');
	await expect(before).toBeFocused();
	await expect(key(page, '1 : Audio')).toBeVisible();

	await after.focus();
	await page.keyboard.press('Enter');
	await expect(page.getByText('Touche déplacée en position 2 sur 5.')).toBeAttached();
	await expect(after).toBeFocused();
	await expect(key(page, '2 : Audio')).toHaveAttribute('aria-current', 'true');
	await expect(key(page, '1 : Pomodoro')).toBeVisible();
});

test('déplace une touche à la souris', async ({ page }) => {
	await open(page);
	await key(page, '1 : Audio').dragTo(key(page, '3 : Docker'));
	await expect(key(page, '3 : Audio')).toBeVisible();
	await expect(page.getByText('Touche déplacée en position 3 sur 5.')).toBeAttached();
});

test('refuse un réglage invalide, puis applique et conserve un réglage valide', async ({
	page
}) => {
	await open(page);
	await key(page, '2 : Pomodoro').click();
	const work = page.getByLabel('Durée du travail');
	await work.fill('0');
	await page.getByRole('button', { name: 'Appliquer' }).click();
	await expect(work).toHaveAttribute('aria-invalid', 'true');
	await expect(work).toBeFocused();
	await expect(work).toHaveAccessibleDescription(/min Entre 1 et 120\. Valeur minimale : 1\./);

	await work.fill('30');
	await page.getByLabel('Libellé').fill('Focus');
	await page.getByRole('button', { name: 'Appliquer' }).click();
	await expect(page.getByText('Réglages enregistrés.')).toBeVisible();
	await expect(key(page, '2 : Focus')).toBeVisible();

	await page.reload();
	await key(page, '2 : Focus').click();
	await expect(page.getByLabel('Durée du travail')).toHaveValue('30');
});

test('refuse un chemin relatif comme le gestionnaire', async ({ page }) => {
	await open(page);
	await key(page, '3 : Docker').click();
	const files = page.getByLabel('Fichiers compose');
	await files.fill('relative/compose.yaml');
	await page.getByRole('button', { name: 'Appliquer' }).click();
	await expect(files).toBeFocused();
	await expect(files).toHaveAccessibleDescription(/« relative\/compose\.yaml » : chemin absolu/);
});

test('garde la saisie quand le gestionnaire refuse la modification', async ({ page }) => {
	// Answers the next update with a refusal, as the manager would for settings it rejects.
	await page.routeWebSocket('**/ws', (socket: WebSocketRoute) => {
		const server = socket.connectToServer();
		socket.onMessage((message) => {
			if (JSON.parse(String(message)).type === 'updateConfig') {
				socket.send(
					JSON.stringify({ type: 'error', code: 'invalidConfig', message: 'refused for the test' })
				);
			} else {
				server.send(message);
			}
		});
	});
	await open(page);
	await key(page, '2 : Pomodoro').click();
	await page.getByLabel('Libellé').fill('Focus');
	await page.getByRole('button', { name: 'Appliquer' }).click();

	const refusal = page.getByRole('alert').filter({ hasText: 'Réglages non enregistrés' });
	await expect(refusal).toContainText('Le gestionnaire a refusé la modification');
	await expect(refusal.locator('[lang="en"]')).toHaveText('refused for the test');
	await expect(page.getByLabel('Libellé')).toHaveValue('Focus');
	await expect(page.getByText('Réglages enregistrés.')).toHaveCount(0);
	await expect(key(page, '2 : Pomodoro')).toBeVisible();
});

test('supprime une touche et place le focus sur sa voisine', async ({ page }) => {
	await open(page);
	await key(page, /^Touche 5 : Claude/).click();
	await page.getByRole('button', { name: 'Supprimer la touche' }).click();
	await expect(page.getByRole('button', { name: /^Touche 5/ })).toHaveCount(0);
	await expect(key(page, /^Touche 4 : IntelliJ/)).toBeFocused();
	await expect(page.getByText('Touche supprimée.')).toBeAttached();
});

test('change le module d’une molette seulement sur « Affecter »', async ({ page }) => {
	await open(page);
	await dial(page, '1 : Horloge').click();
	const module = page.getByLabel('Module', { exact: true });
	await module.focus();
	await page.keyboard.press('ArrowDown');
	await page.keyboard.press('ArrowDown');
	await expect(dial(page, '1 : Horloge')).toBeVisible();
	await expect(page.getByText(/« Horloge » n’est placé nulle part ailleurs/)).toBeVisible();

	await dial(page, '4 : Navigation').click();
	await page.getByLabel('Module', { exact: true }).selectOption({ label: 'Audio (touche 1)' });
	await page.getByRole('button', { name: 'Affecter' }).click();
	await expect(dial(page, '4 : Audio')).toBeVisible();
	await expect(dial(page, '1 : Horloge')).toBeVisible();
	await expect(page.getByLabel('Libellé')).toHaveAttribute('placeholder', 'Audio');
});

test('synchronise les modifications entre deux onglets', async ({ page, context }) => {
	await open(page);
	const other = await context.newPage();
	await open(other);

	await page.getByRole('slider', { name: 'Luminosité' }).fill('35');
	await expect(other.getByRole('slider', { name: 'Luminosité' })).toHaveValue('35');

	// An untouched form follows the other tab; a modified one is kept and flagged.
	await key(other, '2 : Pomodoro').click();
	await key(page, '2 : Pomodoro').click();
	await page.getByLabel('Libellé').fill('Focus');
	await page.getByRole('button', { name: 'Appliquer' }).click();
	await expect(other.getByLabel('Libellé')).toHaveValue('Focus');

	await other.getByLabel('Durée du travail').fill('40');
	await page.getByLabel('Libellé').fill('Zen');
	await page.getByRole('button', { name: 'Appliquer' }).click();
	await expect(other.getByRole('alert')).toContainText('modifié ailleurs');
	await expect(other.getByLabel('Durée du travail')).toHaveValue('40');
	await other.getByRole('button', { name: 'Recharger' }).click();
	await expect(other.getByLabel('Libellé')).toHaveValue('Zen');
	await expect(other.getByLabel('Durée du travail')).toHaveValue('25');
});

test('se reconnecte au gestionnaire en gardant l’éditeur affiché', async ({ page }) => {
	const sockets: WebSocketRoute[] = [];
	await page.routeWebSocket('**/ws', (socket: WebSocketRoute) => {
		sockets.push(socket);
		socket.connectToServer();
	});
	await open(page);

	await sockets[0].close();
	await expect(page.getByRole('alert')).toContainText('Connexion au gestionnaire perdue');
	await expect(key(page, '1 : Audio')).toBeVisible();

	await expect(page.getByRole('status').first()).toHaveText('Connecté au gestionnaire');
	await expect(page.getByRole('alert').filter({ hasText: 'perdue' })).toHaveCount(0);
	// Selected again on the new connection: an edit is saved on the current revision.
	await key(page, '1 : Audio').click();
	await page.getByLabel('Libellé').fill('Volume');
	await page.getByRole('button', { name: 'Appliquer' }).click();
	await expect(page.getByText('Réglages enregistrés.')).toBeVisible();
	await expect(key(page, '1 : Volume')).toBeVisible();
});
