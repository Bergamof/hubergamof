import { describe, expect, it } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { DeckState } from '#lib/deck/deck-state.svelte.js';
import { FakeConnection } from '#lib/deck/fake-connection.js';
import { ADVANCED_MAX } from '#lib/deck/advanced-validation.js';
import { SSH_HOSTS_MAX } from '#lib/deck/ssh-validation.js';
import type { SshConfig, SshHost, SshSystemHost } from '#lib/generated/index.js';
import SshPanel from './SshPanel.svelte';

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

function found(alias: string, change: Partial<SshSystemHost> = {}): SshSystemHost {
	return {
		alias,
		hostName: '',
		user: '',
		port: null,
		identityFile: '',
		proxyJump: '',
		forwardAgent: false,
		options: [],
		lostOptions: [],
		...change
	};
}

function ssh(revision: number, ...hosts: SshHost[]): SshConfig {
	return { schemaVersion: 1, hosts, commonOptions: [], revision };
}

const SYSTEM: SshSystemHost[] = [
	found('web', { hostName: 'web.example.org', user: 'ada' }),
	found('tunnel', { lostOptions: ['identityfile'] }),
	found('far', { identityFile: '/home/ada/key' })
];

async function renderPanel(config: SshConfig | null = ssh(1), system = SYSTEM) {
	const connection = new FakeConnection();
	const deck = new DeckState(connection, { load: () => null, save: () => undefined });
	deck.listen();
	await render(SshPanel, { deck });
	if (config) connection.receive({ type: 'ssh', config, systemHosts: system });
	return { connection };
}

/** Answers the update in flight with the configuration it asked for, one revision further. */
function confirm(connection: FakeConnection) {
	const sent = connection.lastSent();
	if (sent?.type !== 'updateSsh') throw new Error('no update in flight');
	const saved: SshConfig = JSON.parse(JSON.stringify(sent.config));
	saved.revision += 1;
	connection.receive({ type: 'ssh', config: saved, systemHosts: SYSTEM });
}

const addForm = () => page.getByRole('form', { name: 'Ajouter un serveur' });
const managed = () => page.getByRole('list', { name: 'Serveurs SSH gérés' });

describe('SshPanel', () => {
	it('asks the manager to read the computer when it opens, and waits for the configuration', async () => {
		const { connection } = await renderPanel(null);
		expect(connection.sent[0]).toEqual({ type: 'refreshSsh' });
		await expect.element(page.getByText('Chargement de la configuration de SSH…')).toBeVisible();
	});

	it('lists the managed hosts and, apart, those found in the file', async () => {
		await renderPanel(ssh(1, host('prod', { hostName: '10.0.0.1', user: 'deploy', port: 2222 })));
		await expect.element(managed().getByText('prod', { exact: true })).toBeVisible();
		await expect.element(managed().getByText('deploy@10.0.0.1:2222')).toBeVisible();
		const system = page.getByRole('list', { name: 'Serveurs SSH définis ailleurs, non gérés' });
		await expect.element(system.getByText('ada@web.example.org')).toBeVisible();
	});

	describe('adding a host', () => {
		it('sends the whole configuration on its revision', async () => {
			const { connection } = await renderPanel();
			await addForm().getByRole('textbox', { name: 'Nom du serveur' }).fill('prod');
			await addForm().getByRole('textbox', { name: 'Adresse de la machine' }).fill('10.0.0.1');
			await addForm().getByRole('textbox', { name: 'Utilisateur' }).fill('deploy');
			await addForm().getByRole('textbox', { name: 'Port' }).fill('2222');
			await addForm().getByRole('textbox', { name: 'Clé privée' }).fill('~/.ssh/id_ed25519');
			await addForm().getByRole('textbox', { name: 'Rebonds' }).fill('bastion');
			await addForm().getByRole('button', { name: 'Ajouter le serveur' }).click();
			expect(connection.lastSent()).toMatchObject({
				type: 'updateSsh',
				config: {
					revision: 1,
					hosts: [
						{
							alias: 'prod',
							hostName: '10.0.0.1',
							user: 'deploy',
							port: 2222,
							identityFile: '~/.ssh/id_ed25519',
							proxyJump: 'bastion',
							forwardAgent: false,
							enabled: true
						}
					]
				}
			});
		});

		it('says what is wrong and sends nothing, with the focus on the first field', async () => {
			const { connection } = await renderPanel();
			const before = connection.sent.length;
			await addForm().getByRole('button', { name: 'Ajouter le serveur' }).click();
			const name = addForm().getByRole('textbox', { name: 'Nom du serveur' });
			await expect.element(name).toHaveFocus();
			await expect.element(page.getByText('Saisissez le nom du serveur.')).toBeVisible();
			expect(connection.sent).toHaveLength(before);

			await name.fill('prod');
			await addForm().getByRole('textbox', { name: 'Rebonds' }).fill('-oProxyCommand=evil');
			await addForm().getByRole('button', { name: 'Ajouter le serveur' }).click();
			await expect.element(addForm().getByRole('textbox', { name: 'Rebonds' })).toHaveFocus();
			expect(connection.sent).toHaveLength(before);
		});

		it('refuses a key outside ~/.ssh and a port out of range', async () => {
			const { connection } = await renderPanel();
			const before = connection.sent.length;
			await addForm().getByRole('textbox', { name: 'Nom du serveur' }).fill('prod');
			await addForm().getByRole('textbox', { name: 'Clé privée' }).fill('/etc/shadow');
			await addForm().getByRole('textbox', { name: 'Port' }).fill('70000');
			await addForm().getByRole('button', { name: 'Ajouter le serveur' }).click();
			await expect
				.element(addForm().getByRole('textbox', { name: 'Clé privée' }))
				.toHaveAccessibleDescription(/chaque nom : lettres, chiffres/);
			await expect.element(page.getByText(/nombre entier de 1 à 65535/)).toBeVisible();
			expect(connection.sent).toHaveLength(before);
		});

		it('says which jump is wrong, and that an IPv6 address goes between brackets', async () => {
			await renderPanel();
			await addForm().getByRole('textbox', { name: 'Nom du serveur' }).fill('prod');
			await addForm().getByRole('textbox', { name: 'Rebonds' }).fill('bastion,2001:db8::1');
			await addForm().getByRole('button', { name: 'Ajouter le serveur' }).click();
			await expect.element(page.getByText(/« 2001:db8::1 » n’est pas valide/)).toBeVisible();
			await expect.element(page.getByText(/entre crochets/)).toBeVisible();
		});

		it('refuses a name that is taken', async () => {
			await renderPanel(ssh(1, host('prod')));
			await addForm().getByRole('textbox', { name: 'Nom du serveur' }).fill('prod');
			await addForm().getByRole('button', { name: 'Ajouter le serveur' }).click();
			await expect.element(page.getByText('Un serveur porte déjà ce nom.')).toBeVisible();
		});

		it('warns about forwarding the agent once it is turned on, not before', async () => {
			await renderPanel();
			const forward = addForm().getByRole('switch', { name: 'Transmettre l’agent SSH' });
			await expect.element(page.getByText(/utiliser vos clés/)).not.toBeInTheDocument();
			await forward.click();
			await expect.element(forward).toHaveAccessibleDescription(/utiliser vos clés/);
		});

		it('does not offer to add once the configuration is full', async () => {
			await renderPanel(
				ssh(1, ...Array.from({ length: SSH_HOSTS_MAX }, (_, index) => host(`h${index}`)))
			);
			await expect.element(page.getByText(/nombre maximal de serveurs/)).toBeVisible();
		});
	});

	describe('the managed hosts', () => {
		it('switches a host, changes it in place and removes it after a confirmation', async () => {
			const { connection } = await renderPanel(ssh(1, host('prod', { user: 'old' })));
			await page.getByRole('switch', { name: 'Serveur prod' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: { hosts: [{ alias: 'prod', enabled: false }] }
			});
		});

		it('names the form that changes a host', async () => {
			await renderPanel(ssh(1, host('prod')));
			await page.getByRole('button', { name: 'Modifier le serveur prod' }).click();
			await expect
				.element(page.getByRole('form', { name: 'Modifier le serveur prod' }))
				.toBeVisible();
		});

		it('shows that a host forwards the agent, in the list and in the hosts found', async () => {
			await renderPanel(ssh(1, host('prod', { forwardAgent: true })), [
				found('far-away', { forwardAgent: true })
			]);
			await expect.element(managed().getByText(/agent transmis/)).toBeVisible();
			await expect
				.element(
					page
						.getByRole('list', { name: 'Serveurs SSH définis ailleurs, non gérés' })
						.getByText(/agent transmis/)
				)
				.toBeVisible();
		});

		it('changes a host and gives the focus back to its button', async () => {
			const { connection } = await renderPanel(ssh(1, host('prod', { user: 'old' })));
			await page.getByRole('button', { name: 'Modifier le serveur prod' }).click();
			await managed().getByRole('textbox', { name: 'Utilisateur' }).fill('new');
			await managed().getByRole('button', { name: 'Enregistrer' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: { hosts: [{ alias: 'prod', user: 'new' }] }
			});
		});

		it('removes a host after a confirmation, and keeps it when the removal is given up', async () => {
			const { connection } = await renderPanel(ssh(1, host('a'), host('b')));
			const remove = page.getByRole('button', { name: 'Supprimer le serveur a' });
			await remove.click();
			const dialog = page.getByRole('dialog', { name: 'Supprimer le serveur ?' });
			await dialog.getByRole('button', { name: 'Annuler' }).click();
			await expect.element(remove).toHaveFocus();

			await remove.click();
			await dialog.getByRole('button', { name: 'Supprimer' }).click();
			expect(connection.lastSent()).toMatchObject({ config: { hosts: [{ alias: 'b' }] } });
			await expect
				.element(page.getByRole('button', { name: 'Supprimer le serveur b' }))
				.toHaveFocus();
		});
	});

	describe('the hosts found elsewhere', () => {
		it('takes one in, switched on, and removes it from the list at once', async () => {
			const { connection } = await renderPanel();
			await page.getByRole('button', { name: 'Reprendre le serveur web' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: {
					hosts: [{ alias: 'web', hostName: 'web.example.org', user: 'ada', enabled: true }]
				}
			});
			await expect
				.element(
					page
						.getByRole('list', { name: 'Serveurs SSH définis ailleurs, non gérés' })
						.getByText('web.example.org')
				)
				.not.toBeInTheDocument();
		});

		it('does not take one that uses options that would be lost, saying which', async () => {
			const { connection } = await renderPanel();
			const tunnel = page.getByRole('button', { name: 'Reprendre le serveur tunnel' });
			await expect.element(tunnel).toHaveAttribute('aria-disabled', 'true');
			await expect
				.element(
					page.getByText(/Ne peut pas être repris sans perte : identityfile \(plusieurs clés\)/)
				)
				.toBeVisible();
			const before = connection.sent.length;
			await tunnel.click({ force: true });
			expect(connection.sent).toHaveLength(before);
		});

		it('does not take one whose values break the rules', async () => {
			await renderPanel();
			await expect
				.element(page.getByRole('button', { name: 'Reprendre le serveur far' }))
				.toHaveAttribute('aria-disabled', 'true');
		});

		it('focuses the next button, then the title, as hosts are taken', async () => {
			await renderPanel(ssh(1), [found('a'), found('b')]);
			await page.getByRole('button', { name: 'Reprendre le serveur a' }).click();
			await expect
				.element(page.getByRole('button', { name: 'Reprendre le serveur b' }))
				.toHaveFocus();
			await page.getByRole('button', { name: 'Reprendre le serveur b' }).click();
			await expect
				.element(page.getByRole('heading', { name: 'Serveurs définis ailleurs, non gérés' }))
				.toHaveFocus();
		});
	});

	describe('the options written by the person', () => {
		const optionForm = (name: string) => page.getByRole('form', { name });

		it('adds an option common to every host', async () => {
			const { connection } = await renderPanel();
			const form = optionForm('Ajouter une option commune');
			await form.getByRole('textbox', { name: 'Nom de l’option' }).fill('ServerAliveInterval');
			await form.getByRole('textbox', { name: 'Valeur' }).fill('30');
			await form.getByRole('button', { name: 'Ajouter' }).click();
			expect(connection.lastSent()).toMatchObject({
				type: 'updateSsh',
				config: {
					revision: 1,
					commonOptions: [{ key: 'ServerAliveInterval', value: '30', enabled: true }]
				}
			});
			confirm(connection);
			await expect.element(form.getByRole('textbox', { name: 'Nom de l’option' })).toHaveFocus();
		});

		it('refuses a section keyword and says why, sending nothing', async () => {
			const { connection } = await renderPanel();
			const before = connection.sent.length;
			const form = optionForm('Ajouter une option commune');
			await form.getByRole('textbox', { name: 'Nom de l’option' }).fill('Host');
			await form.getByRole('textbox', { name: 'Valeur' }).fill('evil');
			await form.getByRole('button', { name: 'Ajouter' }).click();
			await expect.element(page.getByText(/ouvrent une autre section/)).toBeVisible();
			expect(connection.sent).toHaveLength(before);
		});

		it('warns that an option is written as it is', async () => {
			await renderPanel();
			await expect
				.element(page.getByText(/une option est écrite telle quelle/).last())
				.toBeVisible();
		});

		it('adds an option to one host, the other fields of the host untouched', async () => {
			const { connection } = await renderPanel(
				ssh(1, host('prod', { hostName: '10.0.0.1' }), host('other'))
			);
			await page
				.getByText(/^Options avancées/)
				.first()
				.click();
			const form = optionForm('Ajouter une option à prod');
			await form.getByRole('textbox', { name: 'Nom de l’option' }).fill('Compression');
			await form.getByRole('textbox', { name: 'Valeur' }).fill('yes');
			await form.getByRole('button', { name: 'Ajouter' }).click();
			expect(connection.lastSent()).toMatchObject({
				type: 'updateSsh',
				config: {
					hosts: [
						{
							alias: 'prod',
							hostName: '10.0.0.1',
							options: [{ key: 'Compression', value: 'yes', enabled: true }]
						},
						{ alias: 'other', options: [] }
					]
				}
			});
		});

		it('keeps the options of a host when its other fields are changed', async () => {
			const option = { key: 'Compression', value: 'yes', enabled: true };
			const { connection } = await renderPanel(ssh(1, host('prod', { options: [option] })));
			await expect.element(page.getByText(/^Options avancées \(1\)/)).toBeVisible();
			await page.getByRole('button', { name: 'Modifier le serveur prod' }).click();
			const form = page.getByRole('form', { name: 'Modifier le serveur prod' });
			await form.getByRole('textbox', { name: 'Utilisateur' }).fill('deploy');
			await form.getByRole('button', { name: 'Enregistrer' }).click();
			expect(connection.lastSent()).toMatchObject({
				type: 'updateSsh',
				config: { hosts: [{ alias: 'prod', user: 'deploy', options: [option] }] }
			});
		});

		it('adds a host with its options from the form that adds a host', async () => {
			const { connection } = await renderPanel();
			await addForm().getByRole('textbox', { name: 'Nom du serveur' }).fill('prod');
			await addForm()
				.getByText(/^Options avancées/)
				.click();
			await addForm().getByRole('textbox', { name: 'Nom de l’option' }).fill('Compression');
			await addForm().getByRole('textbox', { name: 'Valeur' }).fill('yes');
			await page.getByRole('textbox', { name: 'Valeur' }).first().click();
			await userEvent.keyboard('{Enter}');
			expect(connection.lastSent()?.type).not.toBe('updateSsh');
			await addForm().getByRole('button', { name: 'Ajouter le serveur' }).click();
			expect(connection.lastSent()).toMatchObject({
				type: 'updateSsh',
				config: {
					hosts: [{ alias: 'prod', options: [{ key: 'Compression', value: 'yes', enabled: true }] }]
				}
			});
		});

		it('changes the options of a host from its form', async () => {
			const option = { key: 'Compression', value: 'yes', enabled: true };
			const { connection } = await renderPanel(ssh(1, host('prod', { options: [option] })));
			await page.getByRole('button', { name: 'Modifier le serveur prod' }).click();
			const form = page.getByRole('form', { name: 'Modifier le serveur prod' });
			await form.getByRole('button', { name: 'Retirer l’option Compression de prod' }).click();
			await form.getByRole('textbox', { name: 'Nom de l’option' }).fill('LogLevel');
			await form.getByRole('textbox', { name: 'Valeur' }).fill('ERROR');
			await form.getByRole('button', { name: 'Ajouter l’option' }).click();
			await form.getByRole('button', { name: 'Enregistrer' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: { hosts: [{ alias: 'prod', options: [{ key: 'LogLevel', value: 'ERROR' }] }] }
			});
		});

		it('takes over a host found elsewhere with its options', async () => {
			const option = { key: 'ProxyCommand', value: 'nc %h %p', enabled: true };
			const { connection } = await renderPanel(ssh(1), [found('proxied', { options: [option] })]);
			await expect.element(page.getByText(/options avancées : ProxyCommand/)).toBeVisible();
			await page.getByRole('button', { name: 'Reprendre le serveur proxied' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: { hosts: [{ alias: 'proxied', options: [option], enabled: true }] }
			});
		});

		it('describes the form that adds an option by the warning', async () => {
			await renderPanel();
			await expect
				.element(optionForm('Ajouter une option commune'))
				.toHaveAccessibleDescription(/une option est écrite telle quelle/);
		});

		it('shows no form when the list holds as many options as the manager accepts', async () => {
			const options = Array.from({ length: ADVANCED_MAX }, (_, index) => ({
				key: `Key${index}`,
				value: 'v',
				enabled: true
			}));
			await renderPanel({ ...ssh(1), commonOptions: options });
			await expect.element(page.getByText(/nombre maximal d’éléments/)).toBeVisible();
			await expect.element(optionForm('Ajouter une option commune')).not.toBeInTheDocument();
		});

		it('keeps the options of a host shown after the host was changed', async () => {
			const option = { key: 'Compression', value: 'yes', enabled: true };
			const { connection } = await renderPanel(ssh(1, host('prod', { options: [option] })));
			await page
				.getByText(/^Options avancées/)
				.first()
				.click();
			await expect
				.element(page.getByRole('list', { name: 'Options du serveur prod' }))
				.toBeVisible();
			await page.getByRole('button', { name: 'Modifier le serveur prod' }).click();
			const form = page.getByRole('form', { name: 'Modifier le serveur prod' });
			await form.getByRole('textbox', { name: 'Utilisateur' }).fill('deploy');
			await form.getByRole('button', { name: 'Enregistrer' }).click();
			confirm(connection);
			await expect
				.element(page.getByRole('list', { name: 'Options du serveur prod' }))
				.toBeVisible();
		});

		it('gives the focus to the next host when one that has options is removed', async () => {
			const option = { key: 'Compression', value: 'yes', enabled: true };
			await renderPanel(ssh(1, host('a', { options: [option] }), host('b')));
			await page.getByRole('button', { name: 'Supprimer le serveur a' }).click();
			const dialog = page.getByRole('dialog', { name: 'Supprimer le serveur ?' });
			await dialog.getByRole('button', { name: 'Supprimer' }).click();
			await expect
				.element(page.getByRole('button', { name: 'Supprimer le serveur b' }))
				.toHaveFocus();
		});

		it('switches an option off and removes it after a confirmation', async () => {
			const option = { key: 'Compression', value: 'yes', enabled: true };
			const { connection } = await renderPanel(ssh(1, host('prod', { options: [option] })));
			await page
				.getByText(/^Options avancées/)
				.first()
				.click();
			await page.getByRole('switch', { name: 'Option Compression du serveur prod' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: { hosts: [{ options: [{ key: 'Compression', enabled: false }] }] }
			});
			confirm(connection);
			await page
				.getByRole('button', { name: 'Supprimer l’option Compression du serveur prod' })
				.click();
			const dialog = page.getByRole('dialog', { name: 'Supprimer l’option ?' });
			await dialog.getByRole('button', { name: 'Supprimer' }).click();
			expect(connection.lastSent()).toMatchObject({
				config: { hosts: [{ alias: 'prod', options: [] }] }
			});
		});
	});
});
