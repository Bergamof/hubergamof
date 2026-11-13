import AxeBuilder from '@axe-core/playwright';
import type { Locator, Page, WebSocketRoute } from '@playwright/test';
import { DEMO_COMPOSE_FILE, expect, test } from './demo-manager.js';

// Each test runs against its own `hubergamof --demo` with the default layout of a Stream Deck+:
// keys Audio, Docker, IntelliJ, Claude, Notifications; dials Volume, Pomodoro (no key: it is driven
// by its dial),
// Contextuelle, Navigation.

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
		'Touche 2 : Docker',
		'Touche 3 : IntelliJ',
		'Touche 4 : Claude',
		'Touche 5 : Notifications'
	]);
	await expect(key(page, '1 : Audio').locator('img')).toHaveAttribute('src', /^data:image\/png/);
	await expectNames(page.getByRole('button', { name: /^Molette / }), [
		'Molette 1 : Contrôle du volume',
		'Molette 2 : Contrôle du pomodoro',
		'Molette 3 : Fonction contextuelle',
		'Molette 4 : Navigation'
	]);
	await expect(page.getByRole('button', { name: /^Bande tactile/ })).toBeVisible();
});

test('ajoute une touche depuis le catalogue, sans proposer ce qui ne s’y place pas', async ({
	page
}) => {
	await open(page);
	await page.getByRole('button', { name: 'Ajouter une touche' }).click();
	const picker = page.getByRole('dialog', { name: 'Ajouter une touche' });
	await expect(picker.getByRole('button', { name: /Horloge/ })).toHaveCount(0);
	// Pomodoro is driven by a dial only; Audio, IntelliJ, Claude and Notifications exist once and
	// are on a key.
	for (const name of [/^Pomodoro/, /^Audio/, /^IntelliJ/, /^Claude/, /^Notifications/]) {
		await expect(picker.getByRole('button', { name })).toHaveCount(0);
	}
	// Docker may be shown again.
	await picker.getByRole('button', { name: /^Docker/ }).click();

	await expect(picker).toBeHidden();
	await expect(key(page, '6 : Docker')).toHaveAttribute('aria-current', 'true');
	await expect(page.getByRole('heading', { name: 'Touche 6 : Docker' })).toBeVisible();
	await expect(page.getByRole('status').filter({ hasText: 'ajoutée' })).toHaveText(
		'Touche Docker ajoutée en position 6.'
	);
	// Next step: configuring it.
	await expect(key(page, '6 : Docker')).toBeFocused();
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
	await expect(key(page, '1 : Docker')).toBeVisible();
});

test('déplace une touche à la souris', async ({ page }) => {
	await open(page);
	await key(page, '1 : Audio').dragTo(key(page, '3 : IntelliJ'));
	await expect(key(page, '3 : Audio')).toBeVisible();
	await expect(page.getByText('Touche déplacée en position 3 sur 5.')).toBeAttached();
});

test('refuse un réglage invalide, puis enregistre sans bouton un réglage valide', async ({
	page
}) => {
	await open(page);
	await key(page, '1 : Audio').click();
	const work = page.getByLabel('Pas de la molette');
	await work.fill('0');
	await work.press('Tab');
	await expect(work).toHaveAttribute('aria-invalid', 'true');
	await expect(work).toHaveAccessibleDescription(/% Entre 1 et 25\. Valeur minimale : 1\./);
	await expect(page.getByRole('status').filter({ hasText: 'Non enregistré' })).toContainText(
		'Pas de la molette'
	);

	await work.fill('10');
	await work.press('Enter');
	await expect(work).not.toHaveAttribute('aria-invalid');
	// A text is saved once the typing pauses, without leaving the field.
	await page.getByLabel('Libellé').fill('Focus');
	await expect(page.getByText('Réglages enregistrés.')).toBeVisible();
	await expect(key(page, '1 : Focus')).toBeVisible();

	await page.reload();
	await key(page, '1 : Focus').click();
	await expect(page.getByLabel('Pas de la molette')).toHaveValue('10');
});

test('avertit qu’une valeur invalide est perdue quand on change de touche', async ({ page }) => {
	await open(page);
	await key(page, '1 : Audio').click();
	await page.getByLabel('Pas de la molette').fill('0');
	await key(page, '2 : Docker').click();
	await expect(page.getByRole('alert')).toContainText(
		'Réglages de « Audio » non enregistrés, valeur invalide : Pas de la molette.'
	);
	await key(page, '1 : Audio').click();
	await expect(page.getByLabel('Pas de la molette')).toHaveValue('5');
});

/** A 1×1 PNG, enough for the manager to accept an icon. */
const PIXEL_PNG = Buffer.from(
	'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
	'base64'
);

test('ajoute, modifie puis supprime un fichier compose avec son nom et son icône', async ({
	page
}) => {
	await open(page);
	await key(page, '2 : Docker').click();
	await expect(page.getByText('Aucun fichier ajouté.')).toBeVisible();

	// The demo manager's dialog answers a file at once: it only has to be named.
	await page.getByRole('button', { name: 'Ajouter un fichier…' }).click();
	await expect(page.getByLabel('Fichier', { exact: true })).toHaveValue(DEMO_COMPOSE_FILE);
	await page.getByLabel('Nom', { exact: true }).fill('Projet démo');
	await page
		.getByRole('group', { name: 'Nouveau fichier' })
		.locator('input[type="file"]')
		.setInputFiles({
			name: 'icone.png',
			mimeType: 'image/png',
			buffer: PIXEL_PNG
		});
	await expect(page.getByText('Icône choisie.')).toBeVisible();
	await page.getByRole('button', { name: 'Ajouter à la liste' }).click();
	await expect(page.getByRole('button', { name: 'Modifier Projet démo' })).toBeFocused();
	await expect(page.getByText('Réglages enregistrés.')).toBeVisible();

	await page.reload();
	await key(page, '2 : Docker').click();
	const file = page.getByRole('listitem').filter({ hasText: 'Projet démo' });
	await expect(file).toContainText(DEMO_COMPOSE_FILE);
	await expect(file.locator('img')).toHaveAttribute('src', /\/api\/devices\/.+\/images\/.+\.png$/);

	await page.getByRole('button', { name: 'Modifier Projet démo' }).click();
	await page.getByLabel('Nom', { exact: true }).fill('Démo renommée');
	await page.getByRole('button', { name: 'Rétablir l’icône par défaut' }).click();
	await page.getByRole('button', { name: 'Valider' }).click();
	await expect(page.getByText('Réglages enregistrés.')).toBeVisible();
	await expect(page.getByRole('listitem').filter({ hasText: 'Démo renommée' })).toBeVisible();

	await page.getByRole('button', { name: 'Supprimer Démo renommée' }).click();
	await expect(page.getByText('Aucun fichier ajouté.')).toBeVisible();
	await expect(page.getByRole('button', { name: 'Ajouter un fichier…' })).toBeFocused();
	await expect(page.getByText('Réglages enregistrés.')).toBeVisible();
	await page.reload();
	await key(page, '2 : Docker').click();
	await expect(page.getByText('Aucun fichier ajouté.')).toBeVisible();
});

test('refuse un chemin de fichier compose relatif comme le gestionnaire', async ({ page }) => {
	await open(page);
	await key(page, '2 : Docker').click();
	await page.getByRole('button', { name: 'Ajouter un fichier…' }).click();
	const path = page.getByLabel('Fichier', { exact: true });
	await path.fill('relative/compose.yaml');
	await page.getByRole('button', { name: 'Ajouter à la liste' }).click();
	await expect(path).toBeFocused();
	await expect(path).toHaveAttribute('aria-invalid', 'true');
	await expect(path).toHaveAccessibleDescription(/« relative\/compose\.yaml » : chemin absolu/);
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
	await key(page, '1 : Audio').click();
	await page.getByLabel('Libellé').fill('Focus');
	await page.getByLabel('Libellé').press('Enter');

	const refusal = page.getByRole('alert').filter({ hasText: 'Réglages non enregistrés' });
	await expect(refusal).toContainText('Le gestionnaire a refusé la modification');
	await expect(refusal.locator('[lang="en"]')).toHaveText('refused for the test');
	await expect(page.getByLabel('Libellé')).toHaveValue('Focus');
	await expect(page.getByText('Réglages enregistrés.')).toHaveCount(0);
	await expect(key(page, '1 : Audio')).toBeVisible();
});

/** Removes the selected key, confirming the loss of its settings. */
async function removeSelectedKey(page: Page) {
	await page.getByRole('button', { name: 'Supprimer la touche' }).click();
	await page
		.getByRole('dialog', { name: 'Supprimer la touche ?' })
		.getByRole('button', { name: 'Supprimer' })
		.click();
}

test('supprime une touche après confirmation et place le focus sur sa voisine', async ({
	page
}) => {
	await open(page);
	await key(page, /^Touche 5 : Notifications/).click();
	await page.getByRole('button', { name: 'Supprimer la touche' }).click();
	const confirm = page.getByRole('dialog', { name: 'Supprimer la touche ?' });
	await expect(confirm).toContainText('« Notifications » n’est placé nulle part ailleurs');
	await expect(confirm.getByRole('button', { name: 'Annuler' })).toBeFocused();
	await page.keyboard.press('Escape');
	await expect(confirm).toBeHidden();
	await expect(key(page, /^Touche 5 : Notifications/)).toBeVisible();

	await removeSelectedKey(page);
	await expect(page.getByRole('button', { name: /^Touche 5/ })).toHaveCount(0);
	await expect(key(page, /^Touche 4 : Claude/)).toBeFocused();
	await expect(page.getByText('Touche supprimée.')).toBeAttached();
});

test('change la fonction d’une molette seulement sur « Affecter »', async ({ page }) => {
	await open(page);
	await dial(page, '3 : Fonction contextuelle').click();
	const fonction = page.getByLabel('Fonction', { exact: true });
	await fonction.focus();
	await page.keyboard.press('ArrowUp');
	// Browsing the list changes nothing.
	await expect(dial(page, '3 : Fonction contextuelle')).toBeVisible();
	await page.getByRole('button', { name: 'Affecter' }).click();
	await expect(dial(page, '3 : Navigation')).toBeVisible();

	// A module has one dial at most: what dials 1 and 2 already offer is not proposed.
	await expect(page.getByRole('option', { name: /Pomodoro|Audio/ })).toHaveCount(0);

	// Once its dial lets it go, the module is deleted with its settings, and a new one can be created.
	await dial(page, '2 : Contrôle du pomodoro').click();
	await fonction.selectOption({ label: 'Rien' });
	await expect(page.getByText(/« Pomodoro » n’est placé nulle part ailleurs/)).toBeVisible();
	await page.getByRole('button', { name: 'Affecter' }).click();
	await dial(page, '3 : Navigation').click();
	await fonction.selectOption({ label: 'Contrôle du pomodoro (Pomodoro, nouveau)' });
	await page.getByRole('button', { name: 'Affecter' }).click();
	await expect(dial(page, '3 : Contrôle du pomodoro')).toBeVisible();
	await expect(page.getByLabel('Libellé')).toHaveAttribute('placeholder', 'Pomodoro');
});

test('règle la bande tactile, avec un aperçu qui suit', async ({ page, context }) => {
	await open(page);
	// The theme is the one of the device: the strip preview and the keys follow it.
	const preview = page.locator('svg[data-theme-preview]');
	const surface = () =>
		page
			.locator('[data-device-theme]')
			.first()
			.evaluate((element) => getComputedStyle(element).getPropertyValue('--screen').trim());
	await expect(preview).toHaveAttribute('data-theme-preview', 'light');
	const lightSurface = await surface();
	await page.getByRole('radio', { name: 'Sombre' }).check();
	await expect(preview).toHaveAttribute('data-theme-preview', 'dark');
	expect(await surface()).not.toBe(lightSurface);

	await page.getByRole('button', { name: /^Bande tactile/ }).click();
	await expect(page.getByRole('heading', { name: 'Bande tactile' })).toBeVisible();
	await page.getByLabel('Afficher les secondes').uncheck();
	await expect(preview.getByText('42', { exact: true })).toHaveCount(0);
	await page.getByLabel(/Actualisation des mesures/).selectOption('5');

	// The edits are sent one after the other: a second tab shows the last one once saved.
	const other = await context.newPage();
	await open(other);
	await other.getByRole('button', { name: /^Bande tactile/ }).click();
	await expect(other.getByLabel(/Actualisation des mesures/)).toHaveValue('5');
	await other.close();

	// Kept by the manager.
	await page.reload();
	await expect(key(page, '1 : Audio')).toBeVisible();
	await page.getByRole('button', { name: /^Bande tactile/ }).click();
	await expect(page.getByRole('radio', { name: 'Sombre' })).toBeChecked();
	await expect(page.getByLabel('Afficher les secondes')).not.toBeChecked();
	await expect(page.getByLabel(/Actualisation des mesures/)).toHaveValue('5');
});

test('synchronise les modifications entre deux onglets', async ({ page, context }) => {
	await open(page);
	const other = await context.newPage();
	await open(other);

	await page.getByRole('slider', { name: 'Luminosité' }).fill('35');
	await expect(other.getByRole('slider', { name: 'Luminosité' })).toHaveValue('35');

	// The fields untouched follow the other tab; one being typed is kept, then saved.
	await key(other, '1 : Audio').click();
	await key(page, '1 : Audio').click();
	await page.getByLabel('Libellé').fill('Focus');
	await page.getByLabel('Libellé').press('Enter');
	await expect(other.getByLabel('Libellé')).toHaveValue('Focus');

	await other.getByLabel('Pas de la molette').fill('15');
	await page.getByLabel('Libellé').fill('Zen');
	await page.getByLabel('Libellé').press('Enter');
	await expect(other.getByLabel('Libellé')).toHaveValue('Zen');
	await expect(other.getByLabel('Pas de la molette')).toHaveValue('15');
	await expect(page.getByLabel('Pas de la molette')).toHaveValue('15');
	await expect(page.getByLabel('Libellé')).toHaveValue('Zen');
});

test('se reconnecte au gestionnaire en gardant l’éditeur affiché', async ({ page }) => {
	const sockets: WebSocketRoute[] = [];
	await page.routeWebSocket('**/ws', (socket: WebSocketRoute) => {
		sockets.push(socket);
		socket.connectToServer();
	});
	await open(page);
	await key(page, '1 : Audio').focus();

	await sockets[0].close();
	await expect(page.getByRole('alert')).toContainText('Connexion au gestionnaire perdue');
	await expect(key(page, '1 : Audio')).toBeVisible();

	await expect(page.getByRole('status').first()).toHaveText('Connecté au gestionnaire');
	await expect(key(page, '1 : Audio'), 'focus given back').toBeFocused();
	await expect(page.getByRole('alert').filter({ hasText: 'perdue' })).toHaveCount(0);
	// Selected again on the new connection: an edit is saved on the current revision.
	await key(page, '1 : Audio').click();
	await page.getByLabel('Libellé').fill('Volume');
	await page.getByLabel('Libellé').press('Enter');
	await expect(page.getByText('Réglages enregistrés.')).toBeVisible();
	await expect(key(page, '1 : Volume')).toBeVisible();
});

/** A 2×2 red PNG. */
const RED_PNG = Buffer.from(
	'iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP4z8AARAwQCgAf7gP9i18U1AAAAABJRU5ErkJggg==',
	'base64'
);

function keyImageInput(page: Page): Locator {
	return page.locator('input[type="file"][accept*="image/png"]');
}

test('remplace l’image d’une touche, puis rétablit l’icône du module', async ({ page }) => {
	await open(page);
	await key(page, '1 : Audio').click();
	await keyImageInput(page).setInputFiles({
		name: 'logo.png',
		mimeType: 'image/png',
		buffer: RED_PNG
	});
	await expect(page.getByText('Image enregistrée.')).toBeVisible();
	const tile = key(page, '1 : Audio').locator('img');
	await expect(tile).toHaveAttribute('src', /^\/api\/devices\/.+\/images\/[0-9a-f]+\.png$/);
	await expect.poll(() => tile.evaluate((image: HTMLImageElement) => image.naturalWidth)).toBe(144);

	await page.getByRole('button', { name: 'Rétablir l’icône du module' }).focus();
	await page.keyboard.press('Enter');
	await expect(page.getByText('Icône du module rétablie.')).toBeVisible();
	await expect(page.getByRole('button', { name: 'Choisir une image…' })).toBeFocused();
	await expect(tile).toHaveAttribute('src', /^data:image\/png/);
});

test('refuse un fichier qui n’est pas une image lisible', async ({ page }) => {
	await open(page);
	await key(page, '1 : Audio').click();
	await keyImageInput(page).setInputFiles({
		name: 'faux.png',
		mimeType: 'image/png',
		buffer: Buffer.from('pas une image')
	});
	await expect(page.getByRole('alert')).toContainText('Image refusée');
	await expect(key(page, '1 : Audio').locator('img')).toHaveAttribute('src', /^data:image\/png/);
});

test('exporte la configuration puis la réimporte après confirmation', async ({ page, context }) => {
	await open(page);
	await key(page, '1 : Audio').click();
	await keyImageInput(page).setInputFiles({
		name: 'logo.png',
		mimeType: 'image/png',
		buffer: RED_PNG
	});
	await expect(page.getByText('Image enregistrée.')).toBeVisible();
	const customImage = await key(page, '1 : Audio').locator('img').getAttribute('src');

	const download = page.waitForEvent('download');
	await page.getByRole('link', { name: 'Exporter la configuration' }).click();
	const archive = await (await download).path();
	expect((await download).suggestedFilename()).toMatch(/^hubergamof-.+-\d{4}-\d{2}-\d{2}\.zip$/);

	await key(page, /^Touche 5 : Notifications/).click();
	await removeSelectedKey(page);
	await expect(page.getByRole('button', { name: /^Touche 5/ })).toHaveCount(0);
	// Importing while the removal is still being saved would race with it: wait until saved.
	const other = await context.newPage();
	await open(other);
	await expect(other.getByRole('button', { name: /^Touche 5/ })).toHaveCount(0);
	await other.close();

	const importInput = page.locator('input[type="file"][accept*=".zip"]');
	await importInput.setInputFiles(archive);
	const confirm = page.getByRole('dialog', { name: /^Remplacer la configuration\s\?$/ });
	await expect(confirm.getByRole('button', { name: 'Annuler' })).toBeFocused();
	await expect(confirm).toContainText('fichiers de confiance');
	await page.keyboard.press('Escape');
	await expect(confirm).toBeHidden();
	await expect(page.getByRole('button', { name: 'Importer une configuration…' })).toBeFocused();
	await expect(page.getByRole('button', { name: /^Touche 5/ })).toHaveCount(0);

	await importInput.setInputFiles(archive);
	await confirm.getByRole('button', { name: 'Remplacer' }).click();
	await expect(page.getByText(/Configuration importée depuis/)).toBeVisible();
	await expect(key(page, /^Touche 5 : Notifications/)).toBeVisible();
	await expect(key(page, '1 : Audio').locator('img')).toHaveAttribute('src', customImage ?? '');
});

test('refuse un fichier qui n’est pas un export', async ({ page }) => {
	await open(page);
	await page.locator('input[type="file"][accept*=".zip"]').setInputFiles({
		name: 'notes.zip',
		mimeType: 'application/zip',
		buffer: Buffer.from('pas une archive')
	});
	await page.getByRole('button', { name: 'Remplacer' }).click();
	await expect(page.getByRole('alert')).toContainText('Configuration non importée');
	await expect(page.getByRole('alert')).toContainText('export HuBergamof');
	await expect(key(page, '1 : Audio')).toBeVisible();
});

/** Fails on any WCAG 2.2 A/AA violation of the page as it is now. */
async function expectAccessible(page: Page, state: string) {
	const results = await new AxeBuilder({ page })
		.withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
		.analyze();
	expect(results.violations, state).toEqual([]);
}

for (const colorScheme of ['light', 'dark'] as const) {
	test(`reste accessible dans ses états riches (thème ${colorScheme})`, async ({ page }) => {
		await page.emulateMedia({ colorScheme });
		await open(page);
		await expectAccessible(page, 'page initiale');

		await key(page, '1 : Audio').click();
		await expectAccessible(page, 'touche sélectionnée');

		await page.getByRole('button', { name: /^Bande tactile/ }).click();
		await expectAccessible(page, 'bande tactile sélectionnée');
		await page.getByRole('radio', { name: 'Sombre' }).check();
		await expectAccessible(page, 'bande tactile sombre');
		await dial(page, '2 : Contrôle du pomodoro').click();
		await expectAccessible(page, 'molette sélectionnée');
		await key(page, '1 : Audio').click();

		const work = page.getByLabel('Pas de la molette');
		await work.fill('0');
		await work.press('Tab');
		await expect(work).toHaveAttribute('aria-invalid', 'true');
		await expectAccessible(page, 'réglages en erreur');

		await page.getByRole('button', { name: 'Ajouter une touche' }).click();
		await expect(page.getByRole('dialog', { name: 'Ajouter une touche' })).toBeVisible();
		await expectAccessible(page, 'sélecteur de modules');
		await page.keyboard.press('Escape');

		await key(page, '2 : Docker').click();
		await page.getByRole('button', { name: 'Ajouter un fichier…' }).click();
		await expect(page.getByLabel('Fichier', { exact: true })).toHaveValue(DEMO_COMPOSE_FILE);
		await expectAccessible(page, 'ajout d’un fichier compose');
		await page.getByRole('button', { name: 'Ajouter à la liste' }).click();
		await expectAccessible(page, 'liste de fichiers compose');

		await key(page, /^Touche 4 : Claude/).click();
		await page.getByRole('button', { name: 'Supprimer la touche' }).click();
		await expect(page.getByRole('dialog', { name: 'Supprimer la touche ?' })).toBeVisible();
		await expectAccessible(page, 'confirmation de suppression');
	});
}
