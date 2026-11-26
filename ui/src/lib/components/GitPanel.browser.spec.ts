import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import { emptyGit } from '#lib/deck/git-test-support.js';
import { GIT_ALIASES_MAX, GIT_FOLDER_IDENTITIES_MAX } from '#lib/deck/git-validation.js';
import type { GitConfig, GitSystemEntry } from '#lib/generated/index.js';
import GitPanel from './GitPanel.svelte';

const FOUND: GitSystemEntry[] = [
	{ key: 'user.name', value: 'Existing Name', origin: '~/.gitconfig' },
	{ key: 'alias.gs', value: 'status --short', origin: '~/.gitconfig' },
	{ key: 'alias.sh', value: '!echo hi', origin: '~/.gitconfig' },
	{ key: 'alias.ro', value: 'log --oneline', origin: '/etc/gitconfig' }
];

function git(revision: number, change: (config: GitConfig) => void = () => undefined): GitConfig {
	const config = emptyGit(revision);
	change(config);
	return config;
}

/** The tab the panel opens on in the test (the one of the identity when none is asked). */
let startTab: string | RegExp | undefined;
afterEach(() => (startTab = undefined));

const openTab = (name: string | RegExp) => page.getByRole('tab', { name }).click();

async function renderPanel(
	config: GitConfig | null = git(1),
	found: GitSystemEntry[] = FOUND,
	available = true
) {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	await render(GitPanel, { deck });
	if (config) {
		connection.receive({ type: 'git', config, systemEntries: found, gitAvailable: available });
	}
	if (startTab && config) await openTab(startTab);
	return { connection };
}

const identityForm = () => page.getByRole('form', { name: 'Identité', exact: true });
const signingForm = () => page.getByRole('form', { name: 'Signature des commits' });
const foldersList = () => page.getByRole('list', { name: 'Identités par dossier' });

/** Answers the update in flight with the configuration it asked for, one revision further. */
function confirm(connection: FakeConnection, change: (config: GitConfig) => void) {
	const sent = connection.lastSent();
	if (sent?.type !== 'updateGit') throw new Error('no update in flight');
	const saved: GitConfig = JSON.parse(JSON.stringify(sent.config));
	saved.revision += 1;
	change(saved);
	connection.receive({ type: 'git', config: saved, systemEntries: FOUND, gitAvailable: true });
}

describe('GitPanel', () => {
	it('asks the manager to read the computer when it opens', async () => {
		const { connection } = await renderPanel();
		expect(connection.sent[0]).toEqual({ type: 'refreshGit' });
	});

	it('waits for the configuration', async () => {
		await renderPanel(null);
		await expect.element(page.getByText('Chargement de la configuration de Git…')).toBeVisible();
	});

	it('says when git was not found', async () => {
		await renderPanel(git(1), [], false);
		await expect.element(page.getByRole('alert').getByText(/est introuvable/)).toBeVisible();
		await openTab('Alias');
		await expect
			.element(page.getByText('Non lu : le programme git est introuvable.').first())
			.toBeVisible();
	});

	it('lists what Git finds elsewhere, aliases apart', async () => {
		await renderPanel();
		await openTab('Avancé');
		const found = page.getByRole('list', { name: 'Valeurs de Git définies ailleurs' });
		await expect.element(found.getByText('Existing Name')).toBeVisible();
		await openTab('Alias');
		const aliases = page.getByRole('list', { name: 'Alias Git définis ailleurs, non gérés' });
		await expect.element(aliases.getByText('status --short')).toBeVisible();
	});

	describe('identity', () => {
		it('sends the whole configuration on its revision', async () => {
			const { connection } = await renderPanel();
			await identityForm().getByRole('textbox', { name: 'Nom' }).fill('Ada Lovelace');
			await identityForm().getByRole('textbox', { name: 'Adresse e-mail' }).fill('ada@example.org');
			await page.getByRole('button', { name: 'Enregistrer l’identité' }).click();
			expect(connection.lastSent()).toMatchObject({
				type: 'updateGit',
				config: {
					revision: 1,
					identity: { name: 'Ada Lovelace', email: 'ada@example.org' }
				}
			});
			await expect.element(page.getByText('Enregistrement…').first()).toBeVisible();
			confirm(connection, () => undefined);
			await expect.element(page.getByText('Modifications enregistrées.').first()).toBeVisible();
		});

		it('says what is wrong, sends nothing and focuses the field', async () => {
			const { connection } = await renderPanel();
			const email = identityForm().getByRole('textbox', { name: 'Adresse e-mail' });
			await email.fill('not an address');
			const before = connection.sent.length;
			await page.getByRole('button', { name: 'Enregistrer l’identité' }).click();
			expect(connection.sent).toHaveLength(before);
			await expect.element(page.getByText(/du type nom@domaine/)).toBeVisible();
			await expect.element(email).toHaveFocus();
			await expect.element(email).toBeInvalid();
		});

		it('notes what was typed and not saved yet, until it is the saved value again', async () => {
			await renderPanel();
			const name = identityForm().getByRole('textbox', { name: 'Nom' });
			await name.fill('Ada');
			await expect.element(page.getByText('Modifications non enregistrées.')).toBeVisible();
			await name.fill('');
			await expect
				.element(page.getByText('Modifications non enregistrées.'))
				.not.toBeInTheDocument();
		});

		it('shows the saved identity of another client while nothing was typed', async () => {
			const { connection } = await renderPanel();
			connection.receive({
				type: 'git',
				config: git(2, (config) => (config.identity.name = 'From elsewhere')),
				systemEntries: [],
				gitAvailable: true
			});
			await expect
				.element(identityForm().getByRole('textbox', { name: 'Nom' }))
				.toHaveValue('From elsewhere');
		});
	});

	describe('signature', () => {
		it('shows the key only while commits are signed', async () => {
			await renderPanel();
			expect(signingForm().getByRole('textbox').elements()).toHaveLength(0);
			await page.getByRole('switch', { name: 'Signer mes commits' }).click();
			await expect
				.element(signingForm().getByRole('textbox', { name: 'Clé de signature' }))
				.toBeVisible();
		});

		it('sends the format and the key', async () => {
			const { connection } = await renderPanel();
			await page.getByRole('switch', { name: 'Signer mes commits' }).click();
			await page.getByRole('radio', { name: 'Clé SSH' }).click({ force: true });
			await signingForm()
				.getByRole('textbox', { name: 'Clé de signature' })
				.fill('~/.ssh/id_ed25519.pub');
			await page.getByRole('button', { name: 'Enregistrer la signature' }).click();
			expect(connection.lastSent()).toMatchObject({
				type: 'updateGit',
				config: {
					signing: { enabled: true, format: 'ssh', key: '~/.ssh/id_ed25519.pub' }
				}
			});
		});

		it('refuses a key that would be taken for an option', async () => {
			const { connection } = await renderPanel();
			await page.getByRole('switch', { name: 'Signer mes commits' }).click();
			await signingForm().getByRole('textbox', { name: 'Clé de signature' }).fill('-u');
			const before = connection.sent.length;
			await page.getByRole('button', { name: 'Enregistrer la signature' }).click();
			expect(connection.sent).toHaveLength(before);
			await expect
				.element(page.getByText(/ne commence ni par une espace ni par « - »/))
				.toBeVisible();
		});
	});

	describe('settings', () => {
		beforeEach(() => (startTab = 'Réglages'));

		it('sends the branch, the way of pulling, the editor and the helper', async () => {
			const { connection } = await renderPanel();
			await page.getByRole('textbox', { name: /Première branche/ }).fill('main');
			await page.getByRole('radio', { name: 'Rebaser' }).click({ force: true });
			await page.getByRole('combobox', { name: /Éditeur/ }).selectOptions('code --wait');
			await page.getByRole('combobox', { name: 'Mots de passe' }).selectOptions('cache');
			await page.getByRole('textbox', { name: /Durée en mémoire/ }).fill('600');
			await page.getByRole('button', { name: 'Enregistrer les réglages' }).click();
			expect(connection.lastSent()).toMatchObject({
				type: 'updateGit',
				config: {
					settings: {
						defaultBranch: 'main',
						pull: 'rebase',
						editor: 'code --wait',
						credentialHelper: { kind: 'cache', timeoutSeconds: 600 }
					}
				}
			});
		});

		it('takes the path of another editor in its own field, and checks it', async () => {
			const { connection } = await renderPanel();
			await page.getByRole('combobox', { name: /Éditeur/ }).selectOptions('other');
			const path = page.getByRole('textbox', { name: 'Chemin du programme' });
			await path.fill('/usr/bin/vim --evil');
			const before = connection.sent.length;
			await page.getByRole('button', { name: 'Enregistrer les réglages' }).click();
			expect(connection.sent).toHaveLength(before);
			await expect.element(page.getByText(/Choisissez un éditeur de la liste/)).toBeVisible();
			await expect.element(path).toHaveFocus();

			await path.fill('/usr/bin/vim');
			await page.getByRole('button', { name: 'Enregistrer les réglages' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: { settings: { editor: '/usr/bin/vim' } }
			});
		});

		it('shows an editor that is not in the list as another program', async () => {
			await renderPanel(git(1, (config) => (config.settings.editor = '/opt/nvim/bin/nvim')));
			await expect.element(page.getByRole('combobox', { name: /Éditeur/ })).toHaveValue('other');
			await expect
				.element(page.getByRole('textbox', { name: 'Chemin du programme' }))
				.toHaveValue('/opt/nvim/bin/nvim');
		});

		it('refuses a duration out of range, and warns about the clear text store', async () => {
			const { connection } = await renderPanel();
			await page.getByRole('combobox', { name: 'Mots de passe' }).selectOptions('cache');
			await page.getByRole('textbox', { name: /Durée en mémoire/ }).fill('5');
			const before = connection.sent.length;
			await page.getByRole('button', { name: 'Enregistrer les réglages' }).click();
			expect(connection.sent).toHaveLength(before);
			await expect.element(page.getByText(/de 30 à 604800/)).toBeVisible();

			await page.getByRole('combobox', { name: 'Mots de passe' }).selectOptions('store');
			await expect.element(page.getByText(/écrits en clair dans/)).toBeVisible();
		});

		it('wants a path once « another program » is chosen', async () => {
			const { connection } = await renderPanel();
			await page.getByRole('combobox', { name: /Éditeur/ }).selectOptions('other');
			const before = connection.sent.length;
			await page.getByRole('button', { name: 'Enregistrer les réglages' }).click();
			expect(connection.sent).toHaveLength(before);
			await expect.element(page.getByText(/Saisissez le chemin du programme/)).toBeVisible();
		});

		it('refuses an interpreter given by its path: it would run what it edits', async () => {
			const { connection } = await renderPanel();
			await page.getByRole('combobox', { name: /Éditeur/ }).selectOptions('other');
			await page.getByRole('textbox', { name: 'Chemin du programme' }).fill('/bin/sh');
			const before = connection.sent.length;
			await page.getByRole('button', { name: 'Enregistrer les réglages' }).click();
			expect(connection.sent).toHaveLength(before);
		});

		it('refuses a branch name Git would not accept', async () => {
			const { connection } = await renderPanel();
			const branch = page.getByRole('textbox', { name: /Première branche/ });
			await branch.fill('a..b');
			const before = connection.sent.length;
			await page.getByRole('button', { name: 'Enregistrer les réglages' }).click();
			expect(connection.sent).toHaveLength(before);
			await expect.element(branch).toHaveFocus();
		});
	});

	describe('aliases', () => {
		beforeEach(() => (startTab = 'Alias'));

		const withAlias = () =>
			git(1, (config) => {
				config.aliases = [{ name: 'co', command: 'checkout', enabled: true }];
			});

		it('adds an alias Git', async () => {
			const { connection } = await renderPanel(withAlias());
			const form = page.getByRole('form', { name: 'Ajouter un alias Git' });
			await form.getByRole('textbox', { name: 'Nom' }).fill('st');
			await form.getByRole('textbox', { name: 'Commande' }).fill('status -s');
			await form.getByRole('button', { name: 'Ajouter' }).click();
			expect(connection.lastSent()).toMatchObject({
				type: 'updateGit',
				config: {
					aliases: [
						{ name: 'co', command: 'checkout', enabled: true },
						{ name: 'st', command: 'status -s', enabled: true }
					]
				}
			});
		});

		it('refuses an alias that runs a shell command, saying why', async () => {
			const { connection } = await renderPanel(withAlias());
			const form = page.getByRole('form', { name: 'Ajouter un alias Git' });
			await form.getByRole('textbox', { name: 'Nom' }).fill('pwn');
			await form.getByRole('textbox', { name: 'Commande' }).fill('!rm -rf ~');
			const before = connection.sent.length;
			await form.getByRole('button', { name: 'Ajouter' }).click();
			expect(connection.sent).toHaveLength(before);
			await expect.element(page.getByText(/ne commence ni par « ! »/)).toBeVisible();
		});

		it('compares names without regard to case', async () => {
			await renderPanel(withAlias());
			const form = page.getByRole('form', { name: 'Ajouter un alias Git' });
			await form.getByRole('textbox', { name: 'Nom' }).fill('CO');
			await form.getByRole('textbox', { name: 'Commande' }).fill('checkout');
			await form.getByRole('button', { name: 'Ajouter' }).click();
			await expect.element(page.getByText(/Un alias porte déjà ce nom/)).toBeVisible();
		});

		it('switches and removes an alias Git', async () => {
			const { connection } = await renderPanel(withAlias());
			await page.getByRole('switch', { name: 'Alias Git co' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: { aliases: [{ name: 'co', enabled: false }] }
			});
			confirm(connection, () => undefined);
			await page.getByRole('button', { name: 'Supprimer l’alias Git co' }).click();
			await page
				.getByRole('dialog', { name: 'Supprimer l’alias Git ?' })
				.getByRole('button', { name: 'Supprimer' })
				.click();
			expect(connection.lastSent()).toMatchObject({ config: { aliases: [] } });
		});

		it('takes an alias found elsewhere in, but not one that runs a shell command', async () => {
			const { connection } = await renderPanel();
			await page.getByRole('button', { name: 'Reprendre l’alias Git gs' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: { aliases: [{ name: 'gs', command: 'status --short', enabled: true }] }
			});

			const shell = page.getByRole('button', { name: 'Reprendre l’alias Git sh' });
			await expect.element(shell).toHaveAttribute('aria-disabled', 'true');
			await expect.element(page.getByText(/lance une commande du shell/)).toBeVisible();
			const before = connection.sent.length;
			await shell.click({ force: true });
			expect(connection.sent).toHaveLength(before);
		});

		it('does not take in an alias defined in a file the manager does not modify', async () => {
			const { connection } = await renderPanel();
			const reprendre = page.getByRole('button', { name: 'Reprendre l’alias Git ro' });
			await expect.element(reprendre).toHaveAttribute('aria-disabled', 'true');
			await expect.element(page.getByText(/Défini dans \/etc\/gitconfig/)).toBeVisible();
			const before = connection.sent.length;
			await reprendre.click({ force: true });
			expect(connection.sent).toHaveLength(before);
		});

		it('does not offer to add an alias once the configuration is full', async () => {
			await renderPanel(
				git(1, (config) => {
					config.aliases = Array.from({ length: GIT_ALIASES_MAX }, (_, index) => ({
						name: `a${index}`,
						command: 'status',
						enabled: true
					}));
				})
			);
			await expect.element(page.getByText(/nombre maximal d’alias Git/)).toBeVisible();
		});
	});

	describe('identities by folder', () => {
		beforeEach(() => (startTab = 'Dossiers'));

		const withFolder = () =>
			git(1, (config) => {
				config.folderIdentities = [
					{ folder: '~/travail', name: '', email: 'ada@work.org', signingKey: '', enabled: true }
				];
			});

		it('adds one', async () => {
			const { connection } = await renderPanel(withFolder());
			const form = page.getByRole('form', { name: 'Ajouter une identité par dossier' });
			await form.getByRole('textbox', { name: 'Dossier' }).fill('~/perso');
			await form.getByRole('textbox', { name: 'Adresse e-mail' }).fill('ada@home.org');
			await form.getByRole('button', { name: 'Ajouter' }).click();
			expect(connection.lastSent()).toMatchObject({
				type: 'updateGit',
				config: {
					folderIdentities: [
						{ folder: '~/travail', email: 'ada@work.org' },
						{ folder: '~/perso', name: '', email: 'ada@home.org', signingKey: '', enabled: true }
					]
				}
			});
		});

		it('refuses a folder outside the home folder, a duplicate and a rule that sets nothing', async () => {
			const { connection } = await renderPanel(withFolder());
			const form = page.getByRole('form', { name: 'Ajouter une identité par dossier' });
			const folder = form.getByRole('textbox', { name: 'Dossier' });
			const before = connection.sent.length;

			await folder.fill('/etc');
			await form.getByRole('button', { name: 'Ajouter' }).click();
			await expect.element(form.getByText(/écrit « ~\/chemin »/)).toBeVisible();
			await expect.element(folder).toHaveFocus();

			await folder.fill('~/travail');
			await form.getByRole('button', { name: 'Ajouter' }).click();
			await expect.element(form.getByText(/existe déjà/)).toBeVisible();

			await folder.fill('~/perso');
			await form.getByRole('button', { name: 'Ajouter' }).click();
			await expect.element(form.getByText(/au moins un nom/)).toBeVisible();
			expect(connection.sent).toHaveLength(before);
		});

		it('changes one in place', async () => {
			const { connection } = await renderPanel(withFolder());
			await page.getByRole('button', { name: 'Modifier l’identité de ~/travail' }).click();
			const email = foldersList().getByRole('textbox', { name: 'Adresse e-mail' });
			await email.fill('ada@corp.org');
			await foldersList().getByRole('button', { name: 'Enregistrer' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: { folderIdentities: [{ folder: '~/travail', email: 'ada@corp.org' }] }
			});
		});

		it('switches one and removes one after a confirmation', async () => {
			const { connection } = await renderPanel(withFolder());
			await page.getByRole('switch', { name: 'Identité de ~/travail' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: { folderIdentities: [{ folder: '~/travail', enabled: false }] }
			});
			confirm(connection, () => undefined);

			await page.getByRole('button', { name: 'Supprimer l’identité de ~/travail' }).click();
			await page
				.getByRole('dialog', { name: 'Supprimer l’identité du dossier ?' })
				.getByRole('button', { name: 'Supprimer' })
				.click();
			expect(connection.lastSent()).toMatchObject({ config: { folderIdentities: [] } });
			// The rule that had the focus is gone: it goes to the form to add one.
			await expect
				.element(
					page
						.getByRole('form', { name: 'Ajouter une identité par dossier' })
						.getByRole('textbox', { name: 'Dossier' })
				)
				.toHaveFocus();
		});

		it('puts the focus on the neighbour when a rule is removed and others remain', async () => {
			const { connection } = await renderPanel(
				git(1, (config) => {
					config.folderIdentities = ['~/a', '~/b'].map((folder) => ({
						folder,
						name: '',
						email: 'x@y.z',
						signingKey: '',
						enabled: true
					}));
				})
			);
			await page.getByRole('button', { name: 'Supprimer l’identité de ~/a' }).click();
			await page
				.getByRole('dialog', { name: 'Supprimer l’identité du dossier ?' })
				.getByRole('button', { name: 'Supprimer' })
				.click();
			expect(connection.lastSent()).toMatchObject({
				config: { folderIdentities: [{ folder: '~/b' }] }
			});
			await expect
				.element(page.getByRole('button', { name: 'Supprimer l’identité de ~/b' }))
				.toHaveFocus();
		});

		it('keeps the focus where it was when the removal is given up', async () => {
			await renderPanel(withFolder());
			const remove = page.getByRole('button', { name: 'Supprimer l’identité de ~/travail' });
			await remove.click();
			await page
				.getByRole('dialog', { name: 'Supprimer l’identité du dossier ?' })
				.getByRole('button', { name: 'Annuler' })
				.click();
			await expect.element(remove).toHaveFocus();
		});

		it('does not offer to add one once the configuration is full', async () => {
			await renderPanel(
				git(1, (config) => {
					config.folderIdentities = Array.from(
						{ length: GIT_FOLDER_IDENTITIES_MAX },
						(_, index) => ({
							folder: `~/f${index}`,
							name: '',
							email: 'a@b.c',
							signingKey: '',
							enabled: true
						})
					);
				})
			);
			await expect.element(page.getByText(/nombre maximal d’identités par dossier/)).toBeVisible();
		});
	});

	describe('the settings written by the person', () => {
		beforeEach(() => (startTab = 'Avancé'));

		const form = () => page.getByRole('form', { name: 'Ajouter une option Git' });

		it('adds a setting and keeps the focus on the key for the next one', async () => {
			const { connection } = await renderPanel();
			await form().getByRole('textbox', { name: 'Clé Git' }).fill('push.default');
			await form().getByRole('textbox', { name: 'Valeur' }).fill('current');
			await form().getByRole('button', { name: 'Ajouter' }).click();
			expect(connection.lastSent()).toMatchObject({
				type: 'updateGit',
				config: {
					revision: 1,
					extraSettings: [{ key: 'push.default', value: 'current', enabled: true }]
				}
			});
			confirm(connection, () => undefined);
			await expect.element(form().getByRole('textbox', { name: 'Clé Git' })).toHaveFocus();
		});

		it('refuses an alias, a managed key and a malformed key, and says why', async () => {
			const { connection } = await renderPanel();
			const before = connection.sent.length;
			const key = form().getByRole('textbox', { name: 'Clé Git' });
			await form().getByRole('textbox', { name: 'Valeur' }).fill('!rm -rf ~');
			for (const [text, message] of [
				['alias.x', /propre place/],
				['user.email', /propre champ/],
				['push', /section\.nom/]
			] as const) {
				await key.fill(text);
				await form().getByRole('button', { name: 'Ajouter' }).click();
				await expect.element(form().getByText(message)).toBeVisible();
			}
			expect(connection.sent).toHaveLength(before);
		});

		it('lists the settings, each switched or removed like an alias', async () => {
			await renderPanel(
				git(1, (config) => {
					config.extraSettings = [{ key: 'core.autocrlf', value: 'input', enabled: false }];
				})
			);
			const list = page.getByRole('list', { name: 'Autres options Git' });
			await expect.element(list.getByText('core.autocrlf')).toBeVisible();
			await expect.element(list.getByText('Désactivé')).toBeVisible();
		});

		it('warns that a setting is written as it is', async () => {
			await renderPanel();
			await expect.element(page.getByText(/une option est écrite telle quelle/)).toBeVisible();
		});
	});
});
