import AxeBuilder from '@axe-core/playwright';
import type { BrowserContext, Locator, Page, WebSocketRoute } from '@playwright/test';
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

/** Follows a link of the main navigation. */
async function goTo(page: Page, name: string | RegExp) {
	await page
		.getByRole('navigation', { name: 'Navigation principale' })
		.getByRole('link', { name })
		.click();
}

/** The banner that says the page cannot be used, or the part of it that holds `text`. */
const banner = (page: Page, text: string) => page.getByRole('status').filter({ hasText: text });

async function open(page: Page) {
	await page.goto('/peripheriques/stream-deck');
	await expect(page.getByRole('status').first()).toHaveText('Connecté au gestionnaire');
	await expect(key(page, '1 : Audio')).toBeVisible();
}

test('dit qu’un Stream Deck de l’adresse n’est plus connecté, sans en montrer un autre', async ({
	page
}) => {
	await page.goto('/peripheriques/stream-deck?appareil=ABSENT');
	await expect(page.getByRole('status').filter({ hasText: 'plus connecté' })).toBeVisible();
	await expect(page.getByRole('link', { name: 'Voir les périphériques' })).toBeVisible();
	await expect(key(page, '1 : Audio')).toHaveCount(0);
});

test('dit qu’une souris de l’adresse n’est plus connectée', async ({ page }) => {
	await page.goto('/peripheriques/souris?appareil=ABSENT');
	await expect(page.getByRole('status').filter({ hasText: 'plus connectée' })).toBeVisible();
});

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
	await goTo(page, 'Mises à jour');
	// `--demo` announces a fake release, without network, and installs it only on request.
	await expect(page.getByText('Version 999.0.0 disponible.')).toBeVisible();
	const notes = page.getByRole('link', { name: /Notes de version/ });
	await expect(notes).toHaveAttribute('href', /^https:\/\//);
	await expect(notes).toHaveAttribute('rel', 'noopener noreferrer');
	await expect(
		page.getByRole('switch', { name: 'Installer automatiquement les mises à jour' })
	).not.toBeChecked();

	await page.getByRole('button', { name: 'Mettre à jour maintenant' }).click();
	await expect(page.getByText('Version 999.0.0 installée : le service redémarre.')).toBeVisible();
});

test('coupe la recherche quotidienne et le gestionnaire le confirme', async ({ page }) => {
	await open(page);
	await goTo(page, 'Mises à jour');
	const daily = page.getByRole('switch', { name: 'Vérifier une fois par jour' });
	await expect(daily).toBeChecked();
	await daily.click();
	await expect(daily).not.toBeChecked();
	await expect(page.getByText(/^Vérifiée /)).toBeVisible();
});

test('relance la recherche de mises à jour à la demande', async ({ page }) => {
	await open(page);
	await goTo(page, 'Mises à jour');
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

test('tient sans défilement horizontal sur un écran de 1024 px', async ({ page }) => {
	await page.setViewportSize({ width: 1024, height: 800 });
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
	await goTo(page, /Stream Deck/);
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
	await expect(banner(page, 'Connexion au gestionnaire perdue')).toBeVisible();
	await expect(key(page, '1 : Audio')).toBeVisible();

	await expect(page.getByRole('status').first()).toHaveText('Connecté au gestionnaire');
	await expect(key(page, '1 : Audio'), 'focus given back').toBeFocused();
	await expect(banner(page, 'perdue')).toHaveCount(0);
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

/** What a backup test starts from: a custom image on the first key, and everything saved as an export. */
async function exportBackup(page: Page): Promise<{ archive: string; customImage: string }> {
	await open(page);
	await key(page, '1 : Audio').click();
	await keyImageInput(page).setInputFiles({
		name: 'logo.png',
		mimeType: 'image/png',
		buffer: RED_PNG
	});
	await expect(page.getByText('Image enregistrée.')).toBeVisible();
	const customImage = (await key(page, '1 : Audio').locator('img').getAttribute('src')) ?? '';

	await goTo(page, 'Sauvegarde');
	await expect(page.getByText('Gardez ce fichier privé.')).toBeVisible();
	const download = page.waitForEvent('download');
	await page.getByRole('button', { name: 'Exporter la configuration' }).click();
	const saved = await download;
	expect(saved.suggestedFilename()).toMatch(/^hubergamof-\d{4}-\d{2}-\d{2}\.zip$/);
	await expect(page.getByText(/Sauvegarde exportée/)).toBeVisible();
	return { archive: await saved.path(), customImage };
}

/** Removes the fifth key and waits until it is saved (another page sees it gone). */
async function removeFifthKey(page: Page, context: BrowserContext) {
	await goTo(page, /Stream Deck/);
	await key(page, /^Touche 5 : Notifications/).click();
	await removeSelectedKey(page);
	await expect(page.getByRole('button', { name: /^Touche 5/ })).toHaveCount(0);
	// Importing while the removal is still being saved would race with it: wait until saved.
	const other = await context.newPage();
	await open(other);
	await expect(other.getByRole('button', { name: /^Touche 5/ })).toHaveCount(0);
	await other.close();
}

/** Chooses `archive` on the backup page and shows what importing it would do. */
async function previewBackup(page: Page, archive: string) {
	await goTo(page, 'Sauvegarde');
	await page.locator('input[type="file"][accept*=".zip"]').setInputFiles(archive);
	await expect(
		page.getByRole('heading', { name: '2. Vérifier ce qui sera importé' })
	).toBeFocused();
}

test('exporte la sauvegarde puis l’importe après l’avoir vue', async ({ page, context }) => {
	const { archive, customImage } = await exportBackup(page);
	await removeFifthKey(page, context);

	await previewBackup(page, archive);
	const deck = page.getByRole('checkbox', { name: /^Stream Deck.+\(.+\)$/ });
	await expect(deck).toBeChecked();
	await expect(page.getByText('Remplacée').first()).toBeVisible();
	await expect(page.getByText(/fichiers de confiance/)).toBeVisible();
	await expect(page.getByRole('button', { name: 'Importer 1 section' })).toBeVisible();
	// Nothing is imported by looking.
	await goTo(page, /Stream Deck/);
	await expect(page.getByRole('button', { name: /^Touche 5/ })).toHaveCount(0);

	await previewBackup(page, archive);
	await page.getByRole('button', { name: 'Importer 1 section' }).click();
	await expect(page.getByRole('heading', { name: '3. Résultat' })).toBeFocused();
	await expect(page.getByText('Appliquée').first()).toBeVisible();
	await expect(page.getByText(/Import terminé/)).toBeVisible();
	await expect(page.getByRole('button', { name: /^Restaurer la sauvegarde du/ })).toHaveCount(1);

	await goTo(page, /Stream Deck/);
	await expect(key(page, /^Touche 5 : Notifications/)).toBeVisible();
	await expect(key(page, '1 : Audio').locator('img')).toHaveAttribute('src', customImage);
});

test('annule un import avec son instantané', async ({ page, context }) => {
	const { archive } = await exportBackup(page);
	await removeFifthKey(page, context);

	await previewBackup(page, archive);
	await page.getByRole('button', { name: 'Importer 1 section' }).click();
	await expect(page.getByRole('heading', { name: '3. Résultat' })).toBeVisible();
	await page.getByRole('button', { name: 'Annuler l’import' }).click();
	const confirm = page.getByRole('dialog', { name: /^Remettre l’état d’avant l’import\s\?$/ });
	await expect(confirm.getByRole('button', { name: 'Annuler', exact: true })).toBeFocused();
	await page.keyboard.press('Escape');
	await expect(confirm).toBeHidden();
	await expect(page.getByRole('button', { name: 'Annuler l’import' })).toBeFocused();

	await page.getByRole('button', { name: 'Annuler l’import' }).click();
	await confirm.getByRole('button', { name: 'Remettre l’état d’avant' }).click();
	await expect(page.getByRole('heading', { name: 'Import annulé' })).toBeFocused();
	// The snapshot of the import, and the one of its undoing.
	await expect(page.getByRole('button', { name: /^Restaurer la sauvegarde du/ })).toHaveCount(2);

	await goTo(page, /Stream Deck/);
	await expect(page.getByRole('button', { name: /^Touche 5/ })).toHaveCount(0);
});

test('restaure une sauvegarde automatique depuis la liste, après l’avoir vue', async ({
	page,
	context
}) => {
	const { archive } = await exportBackup(page);
	await expect(page.getByText('Aucune sauvegarde automatique pour l’instant.')).toBeVisible();
	await removeFifthKey(page, context);
	await previewBackup(page, archive);
	await page.getByRole('button', { name: 'Importer 1 section' }).click();
	await expect(page.getByRole('heading', { name: '3. Résultat' })).toBeVisible();

	await page.getByRole('button', { name: /^Restaurer la sauvegarde du/ }).click();
	const confirm = page.getByRole('dialog', { name: /^Restaurer cette sauvegarde\s\?$/ });
	await expect(confirm).toContainText('Remplacée');
	await confirm.getByRole('button', { name: 'Restaurer' }).click();
	await expect(page.getByText(/Sauvegarde automatique restaurée/)).toBeVisible();

	await goTo(page, /Stream Deck/);
	await expect(page.getByRole('button', { name: /^Touche 5/ })).toHaveCount(0);
});

test('refuse un fichier qui n’est pas une sauvegarde', async ({ page }) => {
	await open(page);
	await goTo(page, 'Sauvegarde');
	await page.locator('input[type="file"][accept*=".zip"]').setInputFiles({
		name: 'notes.zip',
		mimeType: 'application/zip',
		buffer: Buffer.from('pas une archive')
	});
	await expect(page.getByRole('alert')).toContainText('Import non effectué');
	await expect(page.getByRole('alert')).toContainText('export HuBergamof');
	await expect(page.getByRole('heading', { name: '1. Choisir le fichier' })).toBeVisible();
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

	test(`reste accessible sur la page Sauvegarde (thème ${colorScheme})`, async ({ page }) => {
		await page.emulateMedia({ colorScheme });
		const { archive } = await exportBackup(page);
		await expectAccessible(page, 'sauvegarde, étape 1');
		await previewBackup(page, archive);
		await expectAccessible(page, 'sauvegarde, aperçu');
		await page.getByRole('checkbox', { name: /^Stream Deck.+\(.+\)$/ }).check();
		await page.getByRole('button', { name: /^Importer \d+ sections?$/ }).click();
		await expect(page.getByRole('heading', { name: '3. Résultat' })).toBeVisible();
		await expectAccessible(page, 'sauvegarde, résultat');
		await page.getByRole('button', { name: /^Restaurer la sauvegarde du/ }).click();
		await expect(page.getByRole('dialog')).toBeVisible();
		await expectAccessible(page, 'restauration d’une sauvegarde automatique');
	});
}

// The demo also lists two simulated mice (MX Master 4, MX Anywhere 3S): their configuration is saved as a
// real mouse's.

async function openMouse(page: Page) {
	await open(page);
	await goTo(page, /MX Master 4/);
	await expect(page.getByRole('heading', { name: 'MX Master 4', level: 1 })).toBeVisible();
}

/** The control of the list under the drawings, which shows its editor. */
async function chooseControl(page: Page, name: string | RegExp) {
	await page.getByRole('main').getByRole('list').first().getByRole('button', { name }).click();
}

test('configure les boutons, le pointeur et les vibrations de la souris, et les retrouve', async ({
	page
}) => {
	await openMouse(page);
	await expect(
		page.getByRole('navigation', { name: 'Navigation principale' }).getByRole('link', {
			name: /MX Master 4/
		})
	).toHaveAttribute('aria-current', 'page');

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
	await chooseControl(page, /Précédent/);
	await expect(page.getByLabel('Raccourci', { exact: true })).toHaveValue('ctrl+shift+m');
	await page.getByRole('tab', { name: 'Haptique' }).click();
	await expect(page.getByRole('radio', { name: 'Forte' })).toBeChecked();
	await expect(page.getByRole('switch', { name: "Activer : Fin d'une étape" })).not.toBeChecked();
});

test('montre la batterie de chaque souris dans la barre latérale, quelle que soit la page', async ({
	page
}) => {
	await open(page);
	const nav = page.getByRole('navigation', { name: 'Navigation principale' });
	await expect(nav.getByRole('link', { name: /MX Master 4/ })).toContainText(/76\s*%/);
	await expect(nav.getByRole('link', { name: /MX Anywhere 3S/ })).toContainText(/18\s*%/);
	// The headphones are not read until their link is open.
	await expect(nav.getByRole('link', { name: /^WH-1000XM4/ })).toContainText('Non lu');
	await page.goto('/');
	await expect(page.getByRole('button', { name: /Lire la batterie/ })).toBeVisible();
});

test('montre les schémas de chaque modèle, avec un repère par bouton', async ({ page }) => {
	await openMouse(page);
	await expect(page.getByRole('group', { name: 'MX Master 4, vue de dessus' })).toBeVisible();
	await expect(
		page.getByRole('group', { name: 'MX Master 4, côté pouce (face gauche)' })
	).toBeVisible();
	await expect(page.locator('.mk')).toHaveCount(9);
	await expect(page.getByRole('tab', { name: 'Haptique' })).toBeVisible();
	await expect(page.getByRole('main').getByText(/76\s*%/)).toBeVisible();

	await goTo(page, /MX Anywhere 3S/);
	await expect(page.getByRole('heading', { name: 'MX Anywhere 3S', level: 1 })).toBeVisible();
	await expect(page.locator('.mk')).toHaveCount(6);
	await expect(page.getByRole('tab', { name: 'Haptique' })).toHaveCount(0);
	await expect(page.getByRole('main').getByText(/18\s*%\s*· en charge/)).toBeVisible();
	await expect(
		page
			.getByRole('main')
			.getByRole('list')
			.first()
			.getByRole('button', { name: /Bouton du dessus/ })
	).toBeVisible();
});

test('choisit un bouton sur le schéma, au pointeur et au clavier', async ({ page }) => {
	await openMouse(page);
	// The zone of the control on the drawing, then its marker with the keyboard.
	await page.locator('.part[data-id="forward"]').dispatchEvent('click');
	await expect(page.getByRole('heading', { name: 'Suivant', level: 2 })).toBeVisible();
	const marker = page.locator('.mk[data-id="back"]');
	await marker.focus();
	await page.keyboard.press('Enter');
	await expect(page.getByRole('heading', { name: 'Précédent', level: 2 })).toBeVisible();
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
	await goTo(page, 'Menu circulaire');
	await expect(page.getByRole('group', { name: 'Aperçu du menu' })).toBeVisible();
}

test('les pages de la souris et du menu tiennent sans défilement horizontal sur 1024 px', async ({
	page
}) => {
	await page.setViewportSize({ width: 1024, height: 800 });
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
	await goTo(page, 'Menu circulaire');
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
		await goTo(page, /MX Anywhere 3S/);
		await expectAccessible(page, 'MX Anywhere 3S');
		await goTo(page, 'Menu circulaire');
		await expect(page.getByTestId('menu-trigger')).toContainText('Bouton de mode de la molette');
		await expectAccessible(page, 'page du menu circulaire');
	});
}

// The pomodoro lives in the application: the page offers it whether or not a device is there, and
// what it shows is the same as on the dial and in the radial menu.

async function openModules(page: Page) {
	await open(page);
	await goTo(page, 'Modules');
	await page
		.getByRole('navigation', { name: 'Modules à régler' })
		.getByRole('link', { name: /Pomodoro/ })
		.click();
	await expect(page.getByRole('heading', { name: 'Pomodoro', level: 2 })).toBeVisible();
}

test('ne liste que les modules qui ont des réglages, et montre les autres sur l’accueil', async ({
	page
}) => {
	await open(page);
	await goTo(page, 'Modules');
	const list = page.getByRole('navigation', { name: 'Modules à régler' });
	await expect(list.getByRole('link', { name: /Pomodoro/ })).toBeVisible();
	await expect(list.getByRole('link', { name: /Claude/ })).toHaveCount(0);
	await goTo(page, /^Accueil$/);
	await expect(page.getByRole('heading', { name: 'Modules', level: 2 })).toBeVisible();
	await expect(page.getByText('Claude', { exact: true })).toBeVisible();
});

test('pilote le pomodoro de l’application depuis la page Modules', async ({ page }) => {
	await openModules(page);
	const status = page.getByTestId('module-status-pomodoro');
	await expect(status).toContainText('Focus 1/4');
	await expect(status).toContainText('prêt');

	await page.getByRole('button', { name: 'Démarrer ou mettre en pause' }).click();
	await expect(status).not.toContainText('prêt');
	await expect(status).not.toContainText('pause');

	await page.getByRole('button', { name: 'Démarrer ou mettre en pause' }).click();
	await expect(status).toContainText('pause');

	await page.getByRole('button', { name: 'Passer', exact: true }).click();
	await expect(status).toContainText('Pause');
	await page.getByRole('button', { name: 'Remettre à zéro', exact: true }).click();
	await expect(status).toContainText('Focus 1/4');
});

test('montre la même chose sur la molette du deck et dans un autre onglet', async ({
	page,
	context
}) => {
	await openModules(page);
	const other = await context.newPage();
	await other.goto('/application/modules?module=pomodoro');

	await page.getByRole('button', { name: 'Démarrer ou mettre en pause' }).click();
	await expect(other.getByTestId('module-status-pomodoro')).not.toContainText('prêt');

	// The key of the dial of the deck follows too: it is the same timer.
	await goTo(page, /Stream Deck/);
	await expect(dial(page, '2 : Contrôle du pomodoro')).toBeVisible();
});

test('enregistre les réglages du pomodoro, pour toute l’application', async ({ page, context }) => {
	await openModules(page);
	const other = await context.newPage();
	await other.goto('/application/modules?module=pomodoro');

	const work = page.getByLabel('Durée du travail');
	await work.fill('30');
	await work.press('Enter');
	await expect(other.getByLabel('Durée du travail')).toHaveValue('30');
});

// The machine: the aliases the demo manager finds in its own startup file (`MachineHub::demo`).

test('intègre un alias du système, qui reste après un rechargement', async ({ page }) => {
	await open(page);
	await goTo(page, 'Alias du shell');
	const system = page.getByRole('list', { name: 'Alias du système, non gérés' });
	const managed = page.getByRole('list', { name: 'Alias gérés' });
	await expect(system.getByText('git status', { exact: true })).toBeVisible();
	await expect(system.getByText('git log --oneline', { exact: true })).toBeVisible();
	await expect(page.getByText('Aucun alias géré pour l’instant.')).toBeVisible();

	await page.getByRole('button', { name: 'Intégrer l’alias gs' }).click();
	await expect(managed.getByText('git status', { exact: true })).toBeVisible();
	await expect(system.getByText('git status', { exact: true })).toHaveCount(0);
	await expect(page.getByRole('button', { name: 'Intégrer l’alias gl' })).toBeFocused();

	await page.reload();
	await expect(managed.getByText('git status', { exact: true })).toBeVisible();
	await expect(system.getByText('git status', { exact: true })).toHaveCount(0);
	await expect(system.getByText('git log --oneline', { exact: true })).toBeVisible();
});

// Git: the demo manager reads, with the real `git`, the configuration of its own home folder
// (`GitConfigHub::demo`: a name, an address and two aliases).

test('Git : lit la configuration trouvée, enregistre l’identité et refuse un alias du shell', async ({
	page
}) => {
	await open(page);
	await goTo(page, 'Git');
	const found = page.getByRole('list', { name: 'Valeurs de Git définies ailleurs' });
	await expect(found.getByText('Ada Lovelace')).toBeVisible();
	const aliases = page.getByRole('list', { name: 'Alias Git définis ailleurs, non gérés' });
	await expect(aliases.getByText('status --short', { exact: true })).toBeVisible();

	const form = page.getByRole('form', { name: 'Identité', exact: true });
	await form.getByRole('textbox', { name: 'Nom' }).fill('Grace Hopper');
	await page.getByRole('button', { name: 'Enregistrer l’identité' }).click();
	await expect(page.getByText('Modifications enregistrées.').first()).toBeVisible();

	const add = page.getByRole('form', { name: 'Ajouter un alias Git' });
	await add.getByRole('textbox', { name: 'Nom' }).fill('pwn');
	await add.getByRole('textbox', { name: 'Commande' }).fill('!rm -rf ~');
	await add.getByRole('button', { name: 'Ajouter' }).click();
	await expect(page.getByText(/ne commence ni par « ! »/)).toBeVisible();

	await page.reload();
	await expect(form.getByRole('textbox', { name: 'Nom' })).toHaveValue('Grace Hopper');
});

// SSH: the demo manager reads the `~/.ssh/config` of its own home folder (`SshConfigHub::demo`: a host
// that can be taken over, one that has a ProxyCommand of its own (taken over with it) and one that has two
// keys, which cannot be carried).

test('SSH : lit les serveurs trouvés, reprend l’un, refuse l’autre, et le retrouve après un rechargement', async ({
	page
}) => {
	await open(page);
	await goTo(page, 'SSH');
	const found = page.getByRole('list', { name: 'Serveurs SSH définis ailleurs, non gérés' });
	const managed = page.getByRole('list', { name: 'Serveurs SSH gérés' });
	await expect(found.getByText('ada@192.0.2.10:2222')).toBeVisible();
	await expect(
		page.getByRole('button', { name: 'Reprendre le serveur demo-keys' })
	).toHaveAttribute('aria-disabled', 'true');

	await page.getByRole('button', { name: 'Reprendre le serveur demo-server' }).click();
	await expect(managed.getByText('ada@192.0.2.10:2222')).toBeVisible();
	await page.getByRole('button', { name: 'Reprendre le serveur demo-tunnel' }).click();
	await expect(managed.getByText(/192\.0\.2\.20/)).toBeVisible();
	await managed.getByText(/^Options avancées \(1\)/).click();
	await expect(
		managed.getByRole('list', { name: 'Options du serveur demo-tunnel' }).getByText('ProxyCommand')
	).toBeVisible();

	await page.reload();
	await expect(managed.getByText('ada@192.0.2.10:2222')).toBeVisible();
	await expect(found.getByText('ada@192.0.2.10:2222')).toHaveCount(0);
});

test('la page Modules reste accessible', async ({ page }) => {
	await openModules(page);
	await expectAccessible(page, 'page des modules');
});

test('désactive les commandes tant que l’ordinateur est verrouillé, la navigation restant utilisable', async ({
	page
}) => {
	const sockets: WebSocketRoute[] = [];
	await page.routeWebSocket('**/ws', (socket: WebSocketRoute) => {
		sockets.push(socket);
		socket.connectToServer();
	});
	await openModules(page);
	const start = page.getByRole('button', { name: 'Démarrer ou mettre en pause' });
	await expect(start).toBeEnabled();
	await start.focus();

	// What the manager says when the computer locks.
	sockets[0].send(JSON.stringify({ type: 'locked', locked: true }));
	await expect(banner(page, 'Ordinateur verrouillé')).toBeVisible();
	await expect(start).toBeDisabled();
	await start.click({ trial: true, timeout: 1000 }).then(
		() => {
			throw new Error('the page can still be used');
		},
		() => undefined
	);

	// The focus did not fall on the page: it is on the main part.
	await expect(page.locator('main')).toBeFocused();

	sockets[0].send(JSON.stringify({ type: 'locked', locked: false }));
	await expect(banner(page, 'verrouillé')).toHaveCount(0);
	await expect(start, 'focus given back').toBeFocused();
	await start.click();
});

// The demo also serves a simulated pair of headphones: the page opens the link when it is shown.

test('règle le son ambiant et l’égaliseur du casque, et libère la liaison', async ({ page }) => {
	await open(page);
	await goTo(page, /^WH-1000XM4/);
	await expect(page.getByRole('heading', { name: /WH-1000XM4/ }).first()).toBeVisible();
	await expect(page.getByText(/Connexion\s*:\s*Connecté/)).toBeVisible();
	await expect(page.getByRole('main').getByText(/\d+\s*%/)).toBeVisible();

	await page.getByRole('radio', { name: 'Son ambiant' }).click();
	await expect(page.getByRole('radio', { name: 'Son ambiant' })).toBeChecked();
	await expect(page.getByRole('slider', { name: 'Niveau du son ambiant' })).toBeVisible();

	await page.getByLabel('Préréglage de l’égaliseur').selectOption('bassBoost');
	await expect(page.getByLabel('Préréglage de l’égaliseur')).toHaveValue('bassBoost');

	await expectAccessible(page, 'page Casque');

	await page.getByRole('button', { name: 'Libérer' }).click();
	await expect(page.getByText(/Libéré/)).toBeVisible();
	await expect(page.getByRole('button', { name: 'Reprendre la connexion' })).toBeFocused();
	await expect(page.getByRole('slider', { name: 'Égaliseur : 1 kHz' })).toBeDisabled();
	await page.getByRole('button', { name: 'Reprendre la connexion' }).click();
	await expect(page.getByText(/Connexion\s*:\s*Connecté/)).toBeVisible();
	await expect(page.getByRole('button', { name: 'Libérer' })).toBeFocused();
});
