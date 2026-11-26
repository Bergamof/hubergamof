import AxeBuilder from '@axe-core/playwright';
import type { Page, WebSocketRoute } from '@playwright/test';
import { expect, test } from '@playwright/test';

// The page of the machine against a manager played by the test: no device is needed, and the
// manager keeps the revision of the configuration as the real one does.

interface Alias {
	name: string;
	command: string;
	enabled: boolean;
}

const SYSTEM_ALIASES = [
	{ name: 'gs', command: 'git status', source: '.bashrc' },
	{ name: 'dc', command: 'docker compose', source: '.zsh_aliases' }
];

const RADIAL_MENU = {
	schemaVersion: 1,
	holdMs: 280,
	animate: true,
	haptics: true,
	entries: [],
	revision: 1
};

/** What the manager played by the test saw and holds. */
interface FakeManager {
	sockets: WebSocketRoute[];
	/** Messages received, by type. */
	received: string[];
	aliases: Alias[];
	revision: number;
}

async function playManager(page: Page, aliases: Alias[] = []): Promise<FakeManager> {
	const manager: FakeManager = { sockets: [], received: [], aliases, revision: 1 };
	const sendMachine = (socket: WebSocketRoute) =>
		socket.send(
			JSON.stringify({
				type: 'machine',
				config: { schemaVersion: 1, aliases: manager.aliases, revision: manager.revision },
				systemAliases: SYSTEM_ALIASES
			})
		);
	await page.routeWebSocket('**/ws', (socket) => {
		manager.sockets.push(socket);
		socket.send(JSON.stringify({ type: 'hello', version: '1.0.0' }));
		socket.send(JSON.stringify({ type: 'radialMenu', config: RADIAL_MENU }));
		socket.onMessage((raw) => {
			const message = JSON.parse(String(raw));
			manager.received.push(message.type);
			if (message.type === 'refreshMachine') sendMachine(socket);
			if (message.type !== 'updateMachine') return;
			if (message.config.revision !== manager.revision) {
				socket.send(JSON.stringify({ type: 'error', code: 'staleRevision', message: 'stale' }));
				return;
			}
			manager.aliases = message.config.aliases;
			manager.revision += 1;
			sendMachine(socket);
		});
	});
	return manager;
}

/** The aliases of the system sit in a closed block. */
async function openSystem(page: Page) {
	await page.getByText(/^Alias présents sur le système, non gérés \(\d+\)$/).click();
}

async function openMachine(page: Page) {
	await page.goto('/poste/alias');
	await expect(page.getByRole('heading', { name: 'Alias du shell' }).first()).toBeVisible();
}

const managed = (page: Page) => page.getByRole('list', { name: 'Alias gérés' });

test('lit le système à l’ouverture de la page seulement, puis à chaque reconnexion', async ({
	page
}) => {
	const manager = await playManager(page);
	await page.goto('/');
	await expect(page.getByRole('status').first()).toHaveText('Connecté au gestionnaire');
	expect(manager.received).not.toContain('refreshMachine');

	await page.getByRole('link', { name: 'Alias du shell' }).click();
	await expect
		.poll(() => manager.received.filter((type) => type === 'refreshMachine'))
		.toHaveLength(1);

	await manager.sockets[0].close();
	await expect(page.getByRole('status').first()).toHaveText('Connecté au gestionnaire');
	await expect
		.poll(() => manager.received.filter((type) => type === 'refreshMachine'))
		.toHaveLength(2);
});

test('ajoute un alias, désactivé puis réactivé, modifié et supprimé', async ({ page }) => {
	const manager = await playManager(page);
	await openMachine(page);
	await expect(page.getByText('Aucun alias géré pour l’instant.')).toBeVisible();

	await page.getByRole('textbox', { name: 'Nom', exact: true }).fill('ll');
	await page.getByRole('textbox', { name: 'Commande', exact: true }).fill('ls -l');
	await page.getByRole('button', { name: 'Ajouter' }).click();
	await expect(page.getByText('Modifications enregistrées.').first()).toBeVisible();
	await expect(managed(page).getByText('ls -l')).toBeVisible();
	expect(manager.aliases).toEqual([{ name: 'll', command: 'ls -l', enabled: true }]);

	await page.getByRole('switch', { name: 'Alias ll' }).uncheck();
	await expect(managed(page).getByText('Désactivé')).toBeVisible();
	expect(manager.aliases[0].enabled).toBe(false);

	await page.getByRole('button', { name: 'Modifier l’alias ll' }).click();
	await managed(page).getByRole('textbox', { name: 'Commande', exact: true }).fill('ls -lah');
	await page.getByRole('button', { name: 'Enregistrer' }).click();
	await expect(managed(page).getByText('ls -lah')).toBeVisible();
	await expect(page.getByRole('button', { name: 'Modifier l’alias ll' })).toBeFocused();
	expect(manager.aliases[0]).toEqual({ name: 'll', command: 'ls -lah', enabled: false });

	await page.getByRole('button', { name: 'Supprimer l’alias ll' }).click();
	await page
		.getByRole('dialog', { name: 'Supprimer l’alias ?' })
		.getByRole('button', { name: 'Supprimer' })
		.click();
	await expect(page.getByText('Aucun alias géré pour l’instant.')).toBeVisible();
	expect(manager.aliases).toEqual([]);
});

test('dit ce qui ne va pas dans le formulaire, au clavier', async ({ page }) => {
	const manager = await playManager(page, [
		{ name: 'gs', command: 'git status -s', enabled: true }
	]);
	await openMachine(page);

	await page.getByRole('button', { name: 'Ajouter' }).click();
	const name = page.getByRole('textbox', { name: 'Nom', exact: true });
	await expect(name).toBeFocused();
	await expect(name).toHaveAccessibleDescription('Saisissez le nom de l’alias.');
	await expect(
		page.getByRole('textbox', { name: 'Commande', exact: true })
	).toHaveAccessibleDescription('Saisissez la commande.');

	await name.fill('gs');
	await page.keyboard.press('Tab');
	await page.keyboard.type('git status');
	await page.keyboard.press('Enter');
	await expect(page.getByText('Un alias porte déjà ce nom.')).toBeVisible();
	expect(manager.received).not.toContain('updateMachine');
});

test('intègre un alias du système, activé, et le retire de la liste des non gérés', async ({
	page
}) => {
	const manager = await playManager(page);
	await openMachine(page);
	await openSystem(page);
	const system = page.getByRole('list', { name: 'Alias du système, non gérés' });
	await expect(system.getByText('Non géré · lu dans ~/.zsh_aliases')).toBeVisible();

	await page.getByRole('button', { name: 'Intégrer l’alias dc' }).click();
	await expect(managed(page).getByText('docker compose')).toBeVisible();
	await expect(system.getByText('docker compose')).toHaveCount(0);
	await expect(system.getByText('git status')).toBeVisible();
	expect(manager.aliases).toEqual([{ name: 'dc', command: 'docker compose', enabled: true }]);
});

test('annonce une modification refusée parce que la configuration a changé ailleurs', async ({
	page
}) => {
	const manager = await playManager(page);
	await openMachine(page);
	manager.revision = 7;
	await openSystem(page);

	await page.getByRole('button', { name: 'Intégrer l’alias gs' }).click();
	await expect(page.getByRole('alert')).toContainText('modifiée ailleurs');
	await expect(page.getByText('Dernière modification refusée.').first()).toBeVisible();
	expect(manager.aliases).toEqual([]);
});

test('désactive les commandes quand l’ordinateur est verrouillé', async ({ page }) => {
	const manager = await playManager(page);
	await openMachine(page);
	manager.sockets[0].send(JSON.stringify({ type: 'locked', locked: true }));
	await expect(page.getByRole('status').filter({ hasText: 'Ordinateur verrouillé' })).toBeVisible();
	// The page stays readable and the navigation usable, but the controls are disabled.
	await expect(page.getByRole('heading', { name: 'Alias du shell' }).first()).toBeVisible();
	await expect(page.getByRole('button', { name: 'Ajouter' })).toBeDisabled();
	await expect(page.getByRole('link', { name: 'Git', exact: true })).toBeEnabled();
});

for (const colorScheme of ['light', 'dark'] as const) {
	test(`reste accessible (thème ${colorScheme})`, async ({ page }) => {
		await page.emulateMedia({ colorScheme });
		await playManager(page, [
			{ name: 'gs', command: 'git status -s', enabled: true },
			{ name: 'll', command: 'ls -l', enabled: false }
		]);
		await openMachine(page);
		const scan = async (state: string) => {
			const results = await new AxeBuilder({ page })
				.withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
				.analyze();
			expect(results.violations, state).toEqual([]);
		};
		await scan('page de la machine');

		await page.getByRole('button', { name: 'Ajouter' }).click();
		await scan('formulaire en erreur');

		await page.getByRole('button', { name: 'Modifier l’alias gs' }).click();
		await scan('alias en modification');
		await page.getByRole('button', { name: 'Annuler' }).click();

		await page.getByRole('button', { name: 'Supprimer l’alias gs' }).click();
		await scan('confirmation de suppression');
	});
}
