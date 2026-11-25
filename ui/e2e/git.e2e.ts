import AxeBuilder from '@axe-core/playwright';
import type { Page, WebSocketRoute } from '@playwright/test';
import { expect, test } from '@playwright/test';

// The Git page against a manager played by the test: no device is needed, and the manager keeps
// the revision of the configuration as the real one does.

interface GitAlias {
	name: string;
	command: string;
	enabled: boolean;
}

interface FakeGit {
	schemaVersion: number;
	identity: { name: string; email: string };
	signing: { enabled: boolean; format: string; key: string };
	settings: {
		defaultBranch: string;
		pull: string;
		editor: string;
		credentialHelper: { kind: string; timeoutSeconds?: number };
	};
	aliases: GitAlias[];
	folderIdentities: {
		folder: string;
		name: string;
		email: string;
		signingKey: string;
		enabled: boolean;
	}[];
	extraSettings: { key: string; value: string; enabled: boolean }[];
	revision: number;
}

const SYSTEM_ENTRIES = [
	{ key: 'user.name', value: 'Existing Name', origin: '~/.gitconfig' },
	{ key: 'alias.gs', value: 'status --short', origin: '~/.gitconfig' },
	{ key: 'alias.sh', value: '!echo hi', origin: '/etc/gitconfig' }
];

const RADIAL_MENU = {
	schemaVersion: 1,
	holdMs: 280,
	animate: true,
	haptics: true,
	entries: [],
	revision: 1
};

interface FakeManager {
	sockets: WebSocketRoute[];
	received: string[];
	config: FakeGit;
}

function emptyGit(): FakeGit {
	return {
		schemaVersion: 1,
		identity: { name: '', email: '' },
		signing: { enabled: false, format: 'openPgp', key: '' },
		settings: {
			defaultBranch: '',
			pull: 'default',
			editor: '',
			credentialHelper: { kind: 'none' }
		},
		aliases: [],
		folderIdentities: [],
		extraSettings: [],
		revision: 1
	};
}

async function playManager(page: Page, config: FakeGit = emptyGit()): Promise<FakeManager> {
	const manager: FakeManager = { sockets: [], received: [], config };
	const sendGit = (socket: WebSocketRoute) =>
		socket.send(
			JSON.stringify({
				type: 'git',
				config: manager.config,
				systemEntries: SYSTEM_ENTRIES,
				gitAvailable: true
			})
		);
	await page.routeWebSocket('**/ws', (socket) => {
		manager.sockets.push(socket);
		socket.send(JSON.stringify({ type: 'hello', version: '1.0.0' }));
		socket.send(JSON.stringify({ type: 'radialMenu', config: RADIAL_MENU }));
		socket.onMessage((raw) => {
			const message = JSON.parse(String(raw));
			manager.received.push(message.type);
			if (message.type === 'refreshGit') sendGit(socket);
			if (message.type !== 'updateGit') return;
			if (message.config.revision !== manager.config.revision) {
				socket.send(JSON.stringify({ type: 'error', code: 'staleRevision', message: 'stale' }));
				return;
			}
			manager.config = { ...message.config, revision: manager.config.revision + 1 };
			sendGit(socket);
		});
	});
	return manager;
}

async function openGit(page: Page) {
	await page.goto('/poste/git');
	await expect(page.getByRole('heading', { name: 'Identité', exact: true })).toBeVisible();
}

const identityForm = (page: Page) => page.getByRole('form', { name: 'Identité', exact: true });

test('ne lit l’ordinateur qu’à l’ouverture de la page Machine, puis à chaque reconnexion', async ({
	page
}) => {
	const manager = await playManager(page);
	await page.goto('/');
	await expect(page.getByRole('status').first()).toHaveText('Connecté au gestionnaire');
	expect(manager.received).not.toContain('refreshGit');

	await page.getByRole('link', { name: 'Git', exact: true }).click();
	await expect.poll(() => manager.received.filter((type) => type === 'refreshGit')).toHaveLength(1);

	await manager.sockets[0].close();
	await expect(page.getByRole('status').first()).toHaveText('Connecté au gestionnaire');
	await expect.poll(() => manager.received.filter((type) => type === 'refreshGit')).toHaveLength(2);
});

test('garde ce qui est saisi quand on regarde les alias du shell puis qu’on revient', async ({
	page
}) => {
	await playManager(page);
	await openGit(page);
	await identityForm(page).getByRole('textbox', { name: 'Nom' }).fill('Ada');
	await page.getByRole('radio', { name: 'Alias du shell' }).check({ force: true });
	await expect(page.getByRole('heading', { name: 'Alias du shell' })).toBeVisible();
	await page.getByRole('radio', { name: 'Git' }).check({ force: true });
	await expect(identityForm(page).getByRole('textbox', { name: 'Nom' })).toHaveValue('Ada');
});

test('enregistre l’identité, la signature et les réglages', async ({ page }) => {
	const manager = await playManager(page);
	await openGit(page);

	await identityForm(page).getByRole('textbox', { name: 'Nom' }).fill('Ada Lovelace');
	await identityForm(page).getByRole('textbox', { name: 'Adresse e-mail' }).fill('ada@example.org');
	await page.getByRole('button', { name: 'Enregistrer l’identité' }).click();
	await expect(page.getByText('Modifications enregistrées.').first()).toBeVisible();
	expect(manager.config.identity).toEqual({ name: 'Ada Lovelace', email: 'ada@example.org' });

	await page.getByRole('switch', { name: 'Signer mes commits' }).check();
	await page.getByRole('radio', { name: 'Clé SSH' }).check({ force: true });
	await page.getByRole('textbox', { name: 'Clé de signature' }).first().fill('~/.ssh/id.pub');
	await page.getByRole('button', { name: 'Enregistrer la signature' }).click();
	await expect.poll(() => manager.config.revision).toBe(3);
	expect(manager.config.signing).toEqual({ enabled: true, format: 'ssh', key: '~/.ssh/id.pub' });

	await page.getByRole('textbox', { name: /Première branche/ }).fill('main');
	await page.getByRole('combobox', { name: 'Mots de passe' }).selectOption('cache');
	await page.getByRole('button', { name: 'Enregistrer les réglages' }).click();
	await expect.poll(() => manager.config.revision).toBe(4);
	expect(manager.config.settings).toMatchObject({
		defaultBranch: 'main',
		credentialHelper: { kind: 'cache', timeoutSeconds: 900 }
	});
});

test('refuse un alias qui lance une commande du shell, au clavier', async ({ page }) => {
	const manager = await playManager(page);
	await openGit(page);

	const form = page.getByRole('form', { name: 'Ajouter un alias Git' });
	await form.getByRole('textbox', { name: 'Nom' }).fill('pwn');
	await page.keyboard.press('Tab');
	await page.keyboard.type('!rm -rf ~');
	await page.keyboard.press('Enter');
	await expect(form.getByRole('textbox', { name: 'Commande' })).toHaveAccessibleDescription(
		/ne commence ni par « ! »/
	);
	expect(manager.received).not.toContain('updateGit');
});

test('ajoute, désactive et supprime un alias Git, et intègre un alias trouvé ailleurs', async ({
	page
}) => {
	const manager = await playManager(page);
	await openGit(page);
	const managed = page.getByRole('list', { name: 'Alias Git gérés' });
	const found = page.getByRole('list', { name: 'Alias Git définis ailleurs, non gérés' });

	const form = page.getByRole('form', { name: 'Ajouter un alias Git' });
	await form.getByRole('textbox', { name: 'Nom' }).fill('co');
	await form.getByRole('textbox', { name: 'Commande' }).fill('checkout');
	await form.getByRole('button', { name: 'Ajouter' }).click();
	await expect(managed.getByText('checkout')).toBeVisible();

	await page.getByRole('switch', { name: 'Alias Git co' }).uncheck();
	await expect(managed.getByText('Désactivé')).toBeVisible();
	expect(manager.config.aliases).toEqual([{ name: 'co', command: 'checkout', enabled: false }]);

	await page.getByRole('button', { name: 'Reprendre l’alias Git gs' }).click();
	await expect(managed.getByText('status --short')).toBeVisible();
	await expect(found.getByText('status --short')).toHaveCount(0);
	await expect(page.getByRole('button', { name: 'Reprendre l’alias Git sh' })).toHaveAttribute(
		'aria-disabled',
		'true'
	);

	await page.getByRole('button', { name: 'Supprimer l’alias Git co' }).click();
	await page
		.getByRole('dialog', { name: 'Supprimer l’alias Git ?' })
		.getByRole('button', { name: 'Supprimer' })
		.click();
	await expect(managed.getByText('checkout')).toHaveCount(0);
});

test('ajoute un réglage libre, refuse un alias déguisé, au clavier', async ({ page }) => {
	const manager = await playManager(page);
	await openGit(page);

	const form = page.getByRole('form', { name: 'Ajouter une option Git' });
	await form.getByRole('textbox', { name: 'Clé Git' }).fill('alias.pwn');
	await form.getByRole('textbox', { name: 'Valeur' }).fill('!rm -rf ~');
	await page.keyboard.press('Enter');
	await expect(form.getByRole('textbox', { name: 'Clé Git' })).toHaveAccessibleDescription(
		/propre place/
	);
	expect(manager.received).not.toContain('updateGit');

	await form.getByRole('textbox', { name: 'Clé Git' }).fill('push.default');
	await form.getByRole('textbox', { name: 'Valeur' }).fill('current');
	await page.keyboard.press('Enter');
	await expect(page.getByText('Modifications enregistrées.').first()).toBeVisible();
	expect(manager.config.extraSettings).toEqual([
		{ key: 'push.default', value: 'current', enabled: true }
	]);

	await page.getByRole('switch', { name: 'Option Git push.default' }).uncheck();
	await expect(
		page.getByRole('list', { name: 'Autres options Git' }).getByText('Désactivé')
	).toBeVisible();
	expect(manager.config.extraSettings[0].enabled).toBe(false);
});

test('ajoute une identité par dossier et refuse un dossier hors du dossier personnel', async ({
	page
}) => {
	const manager = await playManager(page);
	await openGit(page);
	const form = page.getByRole('form', { name: 'Ajouter une identité par dossier' });

	await form.getByRole('textbox', { name: 'Dossier' }).fill('/etc');
	await form.getByRole('button', { name: 'Ajouter' }).click();
	await expect(form.getByRole('textbox', { name: 'Dossier' })).toBeFocused();
	expect(manager.received).not.toContain('updateGit');

	await form.getByRole('textbox', { name: 'Dossier' }).fill('~/travail');
	await form.getByRole('textbox', { name: 'Adresse e-mail' }).fill('ada@work.org');
	await form.getByRole('button', { name: 'Ajouter' }).click();
	const rules = page.getByRole('list', { name: 'Identités par dossier' });
	await expect(rules.getByText('~/travail')).toBeVisible();
	expect(manager.config.folderIdentities).toEqual([
		{ folder: '~/travail', name: '', email: 'ada@work.org', signingKey: '', enabled: true }
	]);
});

test('annonce une modification refusée parce que la configuration a changé ailleurs', async ({
	page
}) => {
	const manager = await playManager(page);
	await openGit(page);
	manager.config.revision = 7;

	await identityForm(page).getByRole('textbox', { name: 'Nom' }).fill('Ada');
	await page.getByRole('button', { name: 'Enregistrer l’identité' }).click();
	await expect(page.getByRole('alert')).toContainText('modifiée ailleurs');
	await expect(page.getByText('Dernière modification refusée.').first()).toBeVisible();
});

test('tient dans 320 px sans défilement horizontal', async ({ page }) => {
	await page.setViewportSize({ width: 320, height: 800 });
	await playManager(page);
	await openGit(page);
	const overflow = await page.evaluate(
		() => document.documentElement.scrollWidth - document.documentElement.clientWidth
	);
	expect(overflow).toBeLessThanOrEqual(0);
});

for (const colorScheme of ['light', 'dark'] as const) {
	test(`reste accessible (thème ${colorScheme})`, async ({ page }) => {
		await page.emulateMedia({ colorScheme });
		const config = emptyGit();
		config.aliases = [{ name: 'co', command: 'checkout', enabled: false }];
		config.signing = { enabled: true, format: 'ssh', key: '~/.ssh/id.pub' };
		config.settings.credentialHelper = { kind: 'store' };
		config.extraSettings = [{ key: 'push.default', value: 'current', enabled: true }];
		config.folderIdentities = [
			{ folder: '~/travail', name: '', email: 'ada@work.org', signingKey: '', enabled: true }
		];
		await playManager(page, config);
		await openGit(page);
		const scan = async (state: string) => {
			const results = await new AxeBuilder({ page })
				.withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
				.analyze();
			expect(results.violations, state).toEqual([]);
		};
		await scan('page Git');

		await page.getByRole('button', { name: 'Enregistrer les réglages' }).click();
		await page.getByRole('button', { name: 'Enregistrer l’identité' }).click();
		await scan('formulaires envoyés');

		await page.getByRole('button', { name: 'Ajouter', exact: true }).first().click();
		await scan('formulaires en erreur');

		await page.getByRole('button', { name: 'Modifier l’identité de ~/travail' }).click();
		await scan('identité en modification');
		await page.getByRole('button', { name: 'Annuler' }).click();

		await page.getByRole('button', { name: 'Supprimer l’alias Git co' }).click();
		await scan('confirmation de suppression');
	});
}
