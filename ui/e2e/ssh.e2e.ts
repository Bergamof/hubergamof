import AxeBuilder from '@axe-core/playwright';
import type { Page, WebSocketRoute } from '@playwright/test';
import { expect, test } from '@playwright/test';

// The SSH page against a manager played by the test: no device is needed, and the manager keeps the
// revision of the configuration as the real one does.

interface SshHost {
	alias: string;
	hostName: string;
	user: string;
	port: number | null;
	identityFile: string;
	proxyJump: string;
	forwardAgent: boolean;
	enabled: boolean;
	options: Setting[];
}

interface Setting {
	key: string;
	value: string;
	enabled: boolean;
}

interface FakeConfig {
	schemaVersion: number;
	hosts: SshHost[];
	commonOptions: Setting[];
	revision: number;
}

const SYSTEM_HOSTS = [
	{
		alias: 'web',
		hostName: 'web.example.org',
		user: 'ada',
		port: null,
		identityFile: '',
		proxyJump: '',
		forwardAgent: false,
		options: [],
		lostOptions: []
	},
	{
		alias: 'proxied',
		hostName: 'proxied.example.org',
		user: '',
		port: null,
		identityFile: '',
		proxyJump: '',
		forwardAgent: false,
		options: [{ key: 'ProxyCommand', value: 'nc %h %p', enabled: true }],
		lostOptions: []
	},
	{
		alias: 'tunnel',
		hostName: '',
		user: '',
		port: null,
		identityFile: '',
		proxyJump: '',
		forwardAgent: false,
		options: [],
		lostOptions: ['identityfile']
	}
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
	config: FakeConfig;
	/** What the manager answers after saving, when it cannot apply: an error as the real one sends it. */
	notApplied?: string;
}

function host(alias: string, change: Partial<SshHost> = {}): SshHost {
	return {
		alias,
		hostName: '',
		user: '',
		port: null,
		identityFile: '',
		proxyJump: '',
		forwardAgent: false,
		enabled: true,
		options: [],
		...change
	};
}

async function playManager(
	page: Page,
	config: FakeConfig = { schemaVersion: 1, hosts: [], commonOptions: [], revision: 1 }
): Promise<FakeManager> {
	const manager: FakeManager = { sockets: [], received: [], config };
	const sendSsh = (socket: WebSocketRoute) =>
		socket.send(JSON.stringify({ type: 'ssh', config: manager.config, systemHosts: SYSTEM_HOSTS }));
	await page.routeWebSocket('**/ws', (socket) => {
		manager.sockets.push(socket);
		socket.send(JSON.stringify({ type: 'hello', version: '1.0.0' }));
		socket.send(JSON.stringify({ type: 'radialMenu', config: RADIAL_MENU }));
		socket.onMessage((raw) => {
			const message = JSON.parse(String(raw));
			manager.received.push(message.type);
			if (message.type === 'refreshSsh') sendSsh(socket);
			if (message.type !== 'updateSsh') return;
			if (message.config.revision !== manager.config.revision) {
				socket.send(JSON.stringify({ type: 'error', code: 'staleRevision', message: 'stale' }));
				return;
			}
			manager.config = { ...message.config, revision: manager.config.revision + 1 };
			sendSsh(socket);
			if (manager.notApplied) {
				socket.send(
					JSON.stringify({ type: 'error', code: 'notApplied', message: manager.notApplied })
				);
			}
		});
	});
	return manager;
}

async function openSsh(page: Page) {
	await page.goto('/poste/ssh');
	await expect(page.getByRole('heading', { name: 'Serveurs SSH', exact: true })).toBeVisible();
}

const addForm = (page: Page) => page.getByRole('form', { name: 'Ajouter un serveur' });
const managed = (page: Page) => page.getByRole('list', { name: 'Serveurs SSH gérés' });

test('ajoute un serveur, le désactive, le modifie et le supprime', async ({ page }) => {
	const manager = await playManager(page);
	await openSsh(page);
	await expect(page.getByText('Aucun serveur géré pour l’instant.')).toBeVisible();

	await addForm(page).getByRole('textbox', { name: 'Nom du serveur' }).fill('prod');
	await addForm(page).getByRole('textbox', { name: 'Adresse de la machine' }).fill('10.0.0.1');
	await addForm(page).getByRole('textbox', { name: 'Port' }).fill('2222');
	await addForm(page).getByRole('button', { name: 'Ajouter le serveur' }).click();
	await expect(page.getByText('Modifications enregistrées.').first()).toBeVisible();
	await expect(managed(page).getByText('10.0.0.1:2222')).toBeVisible();
	expect(manager.config.hosts[0]).toMatchObject({
		alias: 'prod',
		hostName: '10.0.0.1',
		port: 2222
	});

	await page.getByRole('switch', { name: 'Serveur prod' }).uncheck();
	await expect(managed(page).getByText('Désactivé')).toBeVisible();
	expect(manager.config.hosts[0].enabled).toBe(false);

	await page.getByRole('button', { name: 'Modifier le serveur prod' }).click();
	await managed(page).getByRole('textbox', { name: 'Utilisateur' }).fill('deploy');
	await managed(page).getByRole('button', { name: 'Enregistrer' }).click();
	await expect(managed(page).getByText('deploy@10.0.0.1:2222')).toBeVisible();
	await expect(page.getByRole('button', { name: 'Modifier le serveur prod' })).toBeFocused();

	await page.getByRole('button', { name: 'Supprimer le serveur prod' }).click();
	await page
		.getByRole('dialog', { name: 'Supprimer le serveur ?' })
		.getByRole('button', { name: 'Supprimer' })
		.click();
	await expect(page.getByText('Aucun serveur géré pour l’instant.')).toBeVisible();
	expect(manager.config.hosts).toEqual([]);
});

test('ajoute des options communes et des options d’un serveur, refuse « Host »', async ({
	page
}) => {
	const manager = await playManager(page, {
		schemaVersion: 1,
		hosts: [host('prod', { hostName: '10.0.0.1' })],
		commonOptions: [],
		revision: 1
	});
	await openSsh(page);

	const common = page.getByRole('form', { name: 'Ajouter une option commune' });
	await common.getByRole('textbox', { name: 'Nom de l’option' }).fill('Host');
	await common.getByRole('textbox', { name: 'Valeur' }).fill('evil');
	await page.keyboard.press('Enter');
	await expect(
		common.getByRole('textbox', { name: 'Nom de l’option' })
	).toHaveAccessibleDescription(/autre section/);
	expect(manager.received).not.toContain('updateSsh');

	await common.getByRole('textbox', { name: 'Nom de l’option' }).fill('ServerAliveInterval');
	await common.getByRole('textbox', { name: 'Valeur' }).fill('30');
	await page.keyboard.press('Enter');
	await expect(page.getByText('Modifications enregistrées.').first()).toBeVisible();
	expect(manager.config.commonOptions).toEqual([
		{ key: 'ServerAliveInterval', value: '30', enabled: true }
	]);

	await page
		.getByText(/^Options avancées/)
		.first()
		.click();
	const own = page.getByRole('form', { name: 'Ajouter une option à prod' });
	await own.getByRole('textbox', { name: 'Nom de l’option' }).fill('Compression');
	await own.getByRole('textbox', { name: 'Valeur' }).fill('yes');
	await page.keyboard.press('Enter');
	await expect(page.getByText(/^Options avancées \(1\)/)).toBeVisible();
	expect(manager.config.hosts[0].options).toEqual([
		{ key: 'Compression', value: 'yes', enabled: true }
	]);
});

test('montre à l’écran, en français, qu’une option enregistrée n’est pas appliquée', async ({
	page
}) => {
	const manager = await playManager(page);
	manager.notApplied = 'ssh refuses the options of *: Bad configuration option: Nope';
	await openSsh(page);
	const common = page.getByRole('form', { name: 'Ajouter une option commune' });
	await common.scrollIntoViewIfNeeded();
	await common.getByRole('textbox', { name: 'Nom de l’option' }).fill('Nope');
	await common.getByRole('textbox', { name: 'Valeur' }).fill('1');
	await page.keyboard.press('Enter');
	const alert = page.getByRole('alert');
	await expect(alert).toContainText('Enregistré, mais pas appliqué à l’ordinateur');
	await expect(alert).toContainText('Bad configuration option: Nope');
	await expect(alert).toBeInViewport();
});

test('ajoute un serveur avec ses options depuis le formulaire, Entrée ajoute l’option', async ({
	page
}) => {
	const manager = await playManager(page);
	await openSsh(page);
	const form = addForm(page);
	await form.getByRole('textbox', { name: 'Nom du serveur' }).fill('prod');
	await form.getByText(/^Options avancées/).click();
	await form.getByRole('textbox', { name: 'Nom de l’option' }).fill('Compression');
	await page.keyboard.press('Tab');
	await page.keyboard.type('yes');
	await page.keyboard.press('Enter');
	await expect(form.getByRole('list', { name: 'Options' })).toContainText('Compression');
	expect(manager.received).not.toContain('updateSsh');
	await form.getByRole('button', { name: 'Ajouter le serveur' }).click();
	await expect(managed(page).getByText(/Options avancées \(1\)/)).toBeVisible();
	expect(manager.config.hosts[0].options).toEqual([
		{ key: 'Compression', value: 'yes', enabled: true }
	]);
});

test('refuse un rebond qui serait une option de ssh, au clavier', async ({ page }) => {
	const manager = await playManager(page);
	await openSsh(page);
	await addForm(page).getByRole('textbox', { name: 'Nom du serveur' }).fill('pwn');
	await addForm(page).getByRole('textbox', { name: 'Rebonds' }).fill('-oProxyCommand=evil');
	await page.keyboard.press('Enter');
	const jumps = addForm(page).getByRole('textbox', { name: 'Rebonds' });
	await expect(jumps).toBeFocused();
	await expect(jumps).toHaveAccessibleDescription(/\[utilisateur@\]machine\[:port\]/);
	expect(manager.received).not.toContain('updateSsh');
});

test('reprend un serveur trouvé dans ~/.ssh/config avec ses options, sauf celui qu’on perdrait', async ({
	page
}) => {
	const manager = await playManager(page);
	await openSsh(page);
	await expect(page.getByRole('button', { name: 'Reprendre le serveur tunnel' })).toHaveAttribute(
		'aria-disabled',
		'true'
	);
	await page.getByRole('button', { name: 'Reprendre le serveur web' }).click();
	await expect(managed(page).getByText('ada@web.example.org')).toBeVisible();
	expect(manager.config.hosts).toMatchObject([{ alias: 'web', user: 'ada', enabled: true }]);

	await expect(page.getByText(/options avancées : ProxyCommand/)).toBeVisible();
	await page.getByRole('button', { name: 'Reprendre le serveur proxied' }).click();
	await expect(managed(page).getByText(/proxied\.example\.org/)).toBeVisible();
	expect(manager.config.hosts[1]).toMatchObject({
		alias: 'proxied',
		options: [{ key: 'ProxyCommand', value: 'nc %h %p', enabled: true }]
	});
});

test('annonce une modification refusée parce que la configuration a changé ailleurs', async ({
	page
}) => {
	const manager = await playManager(page);
	await openSsh(page);
	manager.config.revision = 7;
	await addForm(page).getByRole('textbox', { name: 'Nom du serveur' }).fill('prod');
	await addForm(page).getByRole('button', { name: 'Ajouter le serveur' }).click();
	await expect(page.getByRole('alert')).toContainText('modifiée ailleurs');
});

test('tient dans 1024 px sans défilement horizontal', async ({ page }) => {
	await page.setViewportSize({ width: 1024, height: 800 });
	await playManager(page, {
		schemaVersion: 1,
		hosts: [
			host('prod', { hostName: 'a-very-long-host-name.example.org', proxyJump: 'me@bastion:22' })
		],
		commonOptions: [],
		revision: 1
	});
	await openSsh(page);
	const overflow = await page.evaluate(
		() => document.documentElement.scrollWidth - document.documentElement.clientWidth
	);
	expect(overflow).toBeLessThanOrEqual(0);
});

for (const colorScheme of ['light', 'dark'] as const) {
	test(`reste accessible (thème ${colorScheme})`, async ({ page }) => {
		await page.emulateMedia({ colorScheme });
		await playManager(page, {
			schemaVersion: 1,
			hosts: [
				host('prod', {
					hostName: '10.0.0.1',
					forwardAgent: true,
					enabled: false,
					options: [{ key: 'Compression', value: 'yes', enabled: true }]
				})
			],
			commonOptions: [{ key: 'ServerAliveInterval', value: '30', enabled: false }],
			revision: 1
		});
		await openSsh(page);
		const scan = async (state: string) => {
			const results = await new AxeBuilder({ page })
				.withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
				.analyze();
			expect(results.violations, state).toEqual([]);
		};
		await scan('page SSH');

		await addForm(page)
			.getByText(/^Options avancées/)
			.click();
		await scan('options avancées dans le formulaire');

		await addForm(page).getByRole('button', { name: 'Ajouter le serveur' }).click();
		await scan('formulaire en erreur');

		await page
			.getByText(/^Options avancées/)
			.first()
			.click();
		await scan('options avancées d’un serveur');

		await page.getByRole('button', { name: 'Modifier le serveur prod' }).click();
		await scan('serveur en modification');
		await page.getByRole('button', { name: 'Annuler' }).click();

		await page.getByRole('button', { name: 'Supprimer le serveur prod' }).click();
		await scan('confirmation de suppression');
	});
}
