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

test('nomme la page d’après l’appareil affiché', async ({ page }) => {
	await open(page);
	await expect(page).toHaveTitle('Stream Deck+ (virtual) · HuBergamof');
});

test('annonce une version plus récente puis simule son installation', async ({ page }) => {
	await open(page);
	// `--demo` announces a fake release, without network, and installs it only on request.
	await expect(page.getByText('Version 999.0.0 disponible.')).toBeVisible();
	const notes = page.getByRole('link', { name: /Notes de version/ });
	await expect(notes).toHaveAttribute('href', /^https:\/\//);
	await expect(notes).toHaveAttribute('rel', 'noopener noreferrer');
	await expect(
		page.getByRole('checkbox', { name: 'Installer automatiquement les mises à jour' })
	).not.toBeChecked();

	await page.getByRole('button', { name: 'Mettre à jour maintenant' }).click();
	await expect(page.getByText('Version 999.0.0 installée : le service redémarre.')).toBeVisible();
});

test('relance la recherche de mises à jour à la demande', async ({ page }) => {
	await open(page);
	await expect(page.getByText('Version 999.0.0 disponible.')).toBeVisible();
	await page.getByRole('button', { name: 'Rechercher des mises à jour' }).click();
	await expect(page.getByText('Version 999.0.0 disponible.')).toBeVisible();
});

test('explique comment déplacer une touche, à la souris comme au clavier', async ({ page }) => {
	await open(page);
	await expect(page.getByText(/Glissez une touche pour la déplacer/)).toBeVisible();
});

test('le texte d’aide revient à la ligne sans élargir le panneau de l’appareil', async ({
	page
}) => {
	await open(page);
	const help = page.getByText(/Glissez une touche pour la déplacer/);
	const grid = page.getByRole('list').filter({ has: key(page, '1 : Audio') });
	const helpBox = await help.boundingBox();
	const gridBox = await grid.boundingBox();
	expect(helpBox?.width).toBeLessThanOrEqual(gridBox?.width ?? 0);
});

test('tient sans défilement horizontal sur un écran de 320 px', async ({ page }) => {
	await page.setViewportSize({ width: 320, height: 800 });
	await open(page);
	const overflow = await page.evaluate(
		() => document.documentElement.scrollWidth - document.documentElement.clientWidth
	);
	expect(overflow).toBeLessThanOrEqual(0);
	await expect(key(page, '1 : Audio')).toBeVisible();
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

test('échange deux molettes à la souris', async ({ page }) => {
	await open(page);
	await dial(page, '1 : Contrôle du volume').dragTo(dial(page, '3 : Fonction contextuelle'));
	await expect(dial(page, '3 : Contrôle du volume')).toBeVisible();
	await expect(dial(page, '1 : Fonction contextuelle')).toBeVisible();
	await expect(page.getByText('Molettes 1 et 3 échangées.')).toBeAttached();
});

test('échange une molette avec sa voisine au clavier', async ({ page }) => {
	await open(page);
	await dial(page, '1 : Contrôle du volume').click();
	await page.getByRole('button', { name: /Échanger avec la molette suivante/ }).click();
	await expect(page.getByText('Molettes 1 et 2 échangées.')).toBeAttached();
	await expect(dial(page, '2 : Contrôle du volume')).toHaveAttribute('aria-current', 'true');
	await expect(dial(page, '1 : Contrôle du pomodoro')).toBeVisible();
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

test('annonce de nouveau deux suppressions identiques de suite', async ({ page }) => {
	await open(page);
	const status = page.getByRole('status').filter({ hasText: /Touche supprimée/ });
	await key(page, /^Touche 5 : Notifications/).click();
	await removeSelectedKey(page);
	await expect(status).toHaveText('Touche supprimée.');
	// Cleared before the next one: a screen reader hears the second as new.
	await key(page, /^Touche 4 : Claude/).click();
	const cleared = page.evaluate(
		() =>
			new Promise<boolean>((resolve) => {
				const region = [...document.querySelectorAll('[role="status"]')].find((node) =>
					node.textContent?.includes('Touche supprimée')
				);
				if (!region) return resolve(false);
				new MutationObserver((_, observer) => {
					if (region.textContent === '') {
						observer.disconnect();
						resolve(true);
					}
				}).observe(region, { childList: true, characterData: true, subtree: true });
			})
	);
	await removeSelectedKey(page);
	expect(await cleared).toBe(true);
	await expect(status).toHaveText('Touche supprimée.');
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
	await expect(page.getByText(/il sera supprimé/)).toBeVisible();
	await page.getByRole('button', { name: 'Affecter' }).click();
	await page.getByRole('button', { name: 'Retirer' }).click();
	await dial(page, '3 : Navigation').click();
	await fonction.selectOption({ label: 'Contrôle du pomodoro' });
	await page.getByRole('button', { name: 'Affecter' }).click();
	await expect(dial(page, '3 : Contrôle du pomodoro')).toBeVisible();
	// The label names a key: a dial has none to edit, only the settings of its module.
	await expect(page.getByLabel('Libellé')).toHaveCount(0);
	await expect(page.getByRole('heading', { name: 'Réglages' })).toBeVisible();
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

// The demo also lists two simulated mice (MX Master 4, MX Anywhere 3S): their configuration is saved as a
// real mouse's.

async function openMouse(page: Page) {
	await open(page);
	await page.getByRole('button', { name: 'Souris' }).click();
	await expect(page.getByRole('heading', { name: 'MX Master 4' })).toBeVisible();
}

/** The control of the list under the drawings, which shows its editor. */
async function chooseControl(page: Page, name: string | RegExp) {
	await page.getByRole('list').first().getByRole('button', { name }).click();
}

test('configure les boutons, le pointeur et les vibrations de la souris, et les retrouve', async ({
	page
}) => {
	await openMouse(page);
	await expect(page.getByRole('button', { name: 'Souris' })).toHaveAttribute(
		'aria-pressed',
		'true'
	);

	await chooseControl(page, /Précédent/);
	await page.getByLabel('Action', { exact: true }).selectOption('shortcut');
	await page.getByLabel('Raccourci', { exact: true }).fill('Ctrl + Maj + M');
	await page.getByLabel('Raccourci', { exact: true }).press('Enter');
	await expect(page.getByText('Modifications enregistrées.').first()).toBeVisible();

	await page.getByRole('tab', { name: 'Haptique' }).click();
	await page.getByRole('radio', { name: 'Forte' }).click();
	await page.getByRole('switch', { name: "Activer : Fin d'une étape" }).uncheck();
	// « Enregistrées » reste affiché depuis la sauvegarde précédente : attendre la fin de celle-ci.
	await expect(page.getByText('Enregistrement…')).toHaveCount(0);
	await expect(page.getByText('Modifications enregistrées.').first()).toBeVisible();

	await page.reload();
	await expect(page.getByRole('status').first()).toHaveText('Connecté au gestionnaire');
	await page.getByRole('button', { name: 'Souris' }).click();
	await chooseControl(page, /Précédent/);
	await expect(page.getByLabel('Raccourci', { exact: true })).toHaveValue('ctrl+shift+m');
	await page.getByRole('tab', { name: 'Haptique' }).click();
	await expect(page.getByRole('radio', { name: 'Forte' })).toBeChecked();
	await expect(page.getByRole('switch', { name: "Activer : Fin d'une étape" })).not.toBeChecked();
});

test('montre les schémas de chaque modèle, avec un repère par bouton', async ({ page }) => {
	await openMouse(page);
	await expect(page.getByRole('group', { name: 'MX Master 4, vue de dessus' })).toBeVisible();
	await expect(
		page.getByRole('group', { name: 'MX Master 4, côté pouce (face gauche)' })
	).toBeVisible();
	await expect(page.locator('.mk')).toHaveCount(9);
	await expect(page.getByRole('tab', { name: 'Haptique' })).toBeVisible();
	await expect(page.getByText('Batterie 76 %', { exact: true })).toBeVisible();

	await page.getByRole('radio', { name: 'MX Anywhere 3S' }).click();
	await expect(page.getByRole('heading', { name: 'MX Anywhere 3S' })).toBeVisible();
	await expect(page.locator('.mk')).toHaveCount(6);
	await expect(page.getByRole('tab', { name: 'Haptique' })).toHaveCount(0);
	await expect(page.getByText('Batterie 18 % · en charge')).toBeVisible();
	await expect(
		page
			.getByRole('list')
			.first()
			.getByRole('button', { name: /Bouton du dessus/ })
	).toBeVisible();
});

test('choisit un bouton sur le schéma, au pointeur et au clavier', async ({ page }) => {
	await openMouse(page);
	// The zone of the control on the drawing, then its marker with the keyboard.
	await page.locator('.part[data-id="forward"]').dispatchEvent('click');
	await expect(page.getByRole('heading', { name: 'Suivant', level: 3 })).toBeVisible();
	const marker = page.locator('.mk[data-id="back"]');
	await marker.focus();
	await page.keyboard.press('Enter');
	await expect(page.getByRole('heading', { name: 'Précédent', level: 3 })).toBeVisible();
	await chooseControl(page, /Clic gauche/);
	await expect(page.getByText(/Ce bouton est verrouillé/)).toBeVisible();
});

test('essaie une vibration depuis l’onglet Haptique et coupe un module entier', async ({
	page
}) => {
	await openMouse(page);
	await page.getByRole('tab', { name: 'Haptique' }).click();
	await expect(page.getByText('Choisissez une onde pour la sentir.')).toBeVisible();
	await page.getByRole('button', { name: "Jouer : Fin d'une étape" }).click();
	await expect(page.getByText(/Joué sur la souris/)).toBeVisible();

	await page.getByLabel("Vibration de : Pomodoro, Fin d'une étape").selectOption('knock');
	await expect(page.getByText('Joué sur la souris : Deux petits coups')).toBeVisible();

	const all = page.getByRole('switch', {
		name: 'Activer ou désactiver tous les retours de Pomodoro'
	});
	await all.uncheck();
	await expect(page.getByRole('button', { name: "Jouer : Fin d'une étape" })).toBeDisabled();
	await expect(page.getByText('Modifications enregistrées.').first()).toBeVisible();

	await page.getByRole('radio', { name: 'Aucune vibration' }).click();
	await expect(page.getByText(/Retour haptique désactivé/)).toBeVisible();
});

async function openMenu(page: Page) {
	await open(page);
	await page.getByRole('button', { name: 'Menu circulaire', exact: true }).click();
	await expect(page.getByRole('group', { name: 'Aperçu du menu' })).toBeVisible();
}

test('les pages de la souris et du menu tiennent sans défilement horizontal sur 320 px', async ({
	page
}) => {
	await page.setViewportSize({ width: 320, height: 800 });
	const overflow = () =>
		page.evaluate(
			() => document.documentElement.scrollWidth - document.documentElement.clientWidth
		);
	await openMouse(page);
	expect(await overflow(), 'onglet Touches').toBeLessThanOrEqual(0);
	await page.getByRole('tab', { name: 'Pointeur et molette' }).click();
	expect(await overflow(), 'onglet Pointeur et molette').toBeLessThanOrEqual(0);
	await page.getByRole('tab', { name: 'Haptique' }).click();
	expect(await overflow(), 'onglet Haptique').toBeLessThanOrEqual(0);
	await page.getByRole('button', { name: 'Menu circulaire', exact: true }).click();
	await expect(page.getByRole('group', { name: 'Aperçu du menu' })).toBeVisible();
	expect(await overflow(), 'menu circulaire').toBeLessThanOrEqual(0);
});

test('compose le menu circulaire et garde le minimum d’entrées', async ({ page }) => {
	await openMenu(page);
	await expect(page.getByText(/4 actuellement/)).toBeVisible();
	await page.getByRole('button', { name: 'Descendre : Pomodoro' }).click();
	await expect(page.getByText('Modifications enregistrées.').first()).toBeVisible();
	await page.getByRole('switch', { name: 'Teams' }).uncheck();
	await expect(page.getByText(/3 actuellement/)).toBeVisible();
	await expect(page.getByRole('switch', { name: 'Pomodoro' })).toBeDisabled();
});

for (const colorScheme of ['light', 'dark'] as const) {
	test(`la page de la souris reste accessible (thème ${colorScheme})`, async ({ page }) => {
		await page.emulateMedia({ colorScheme });
		await openMouse(page);
		await expectAccessible(page, 'onglet Touches');
		await chooseControl(page, /Précédent/);
		await page.getByLabel('Action', { exact: true }).selectOption('shortcut');
		await expectAccessible(page, 'raccourci en cours de saisie');
		await page.getByRole('tab', { name: 'Pointeur et molette' }).click();
		await expectAccessible(page, 'onglet Pointeur et molette');
		await page.getByRole('tab', { name: 'Haptique' }).click();
		await expectAccessible(page, 'onglet Haptique');
		await page.getByRole('radio', { name: 'MX Anywhere 3S' }).click();
		await expectAccessible(page, 'MX Anywhere 3S');
		await page.getByRole('button', { name: 'Menu circulaire', exact: true }).click();
		await expect(page.getByTestId('menu-trigger')).toContainText('Bouton de mode de la molette');
		await expectAccessible(page, 'page du menu circulaire');
	});
}
