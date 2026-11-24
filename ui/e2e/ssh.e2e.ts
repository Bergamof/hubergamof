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
		otherOptions: []
	},
	{
		alias: 'tunnel',
		hostName: '',
		user: '',
		port: null,
		identityFile: '',
		proxyJump: '',
		forwardAgent: false,
		otherOptions: ['proxycommand']
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
		});
	});
	return manager;
}

async function openSsh(page: Page) {
	await page.goto('/');
	await page.getByRole('button', { name: 'Machine' }).click();
	await page.getByRole('radio', { name: 'SSH' }).check({ force: true });
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
	await addForm(page).getByRole('button', { name: 'Ajouter' }).click();
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
	await common.getByRole('textbox', { name: 'Option' }).fill('Host');
	await common.getByRole('textbox', { name: 'Valeur' }).fill('evil');
	await page.keyboard.press('Enter');
	await expect(common.getByRole('textbox', { name: 'Option' })).toHaveAccessibleDescription(
		/autre section/
	);
	expect(manager.received).not.toContain('updateSsh');

	await common.getByRole('textbox', { name: 'Option' }).fill('ServerAliveInterval');
	await common.getByRole('textbox', { name: 'Valeur' }).fill('30');
	await page.keyboard.press('Enter');
	await expect(page.getByText('Modifications enregistrées.').first()).toBeVisible();
	expect(manager.config.commonOptions).toEqual([
		{ key: 'ServerAliveInterval', value: '30', enabled: true }
	]);

	await page.getByText(/^Options avancées/).click();
	const own = page.getByRole('form', { name: 'Ajouter une option à prod' });
	await own.getByRole('textbox', { name: 'Option' }).fill('Compression');
	await own.getByRole('textbox', { name: 'Valeur' }).fill('yes');
	await page.keyboard.press('Enter');
	await expect(page.getByText(/^Options avancées \(1\)/)).toBeVisible();
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

test('reprend un serveur trouvé dans ~/.ssh/config, sauf celui qui lance un programme', async ({
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
});

test('annonce une modification refusée parce que la configuration a changé ailleurs', async ({
	page
}) => {
	const manager = await playManager(page);
	await openSsh(page);
	manager.config.revision = 7;
	await addForm(page).getByRole('textbox', { name: 'Nom du serveur' }).fill('prod');
	await addForm(page).getByRole('button', { name: 'Ajouter' }).click();
	await expect(page.getByRole('alert')).toContainText('modifiée ailleurs');
});

test('tient dans 320 px sans défilement horizontal', async ({ page }) => {
	await page.setViewportSize({ width: 320, height: 800 });
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

		await addForm(page).getByRole('button', { name: 'Ajouter' }).click();
		await scan('formulaire en erreur');

		await page.getByText(/^Options avancées/).click();
		await scan('options avancées d’un serveur');

		await page.getByRole('button', { name: 'Modifier le serveur prod' }).click();
		await scan('serveur en modification');
		await page.getByRole('button', { name: 'Annuler' }).click();

		await page.getByRole('button', { name: 'Supprimer le serveur prod' }).click();
		await scan('confirmation de suppression');
	});
}
