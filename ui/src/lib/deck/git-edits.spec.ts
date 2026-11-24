import { describe, expect, it } from 'vitest';
import type { GitConfig, GitFolderIdentity } from '#lib/generated/index.js';
import { addSetting, removeSetting } from './advanced-edits.js';
import { addAlias, isFull, removeAlias, replaceAlias, setAliasEnabled } from './machine-edits.js';
import {
	addFolderIdentity,
	editExtraSettings,
	isFolderListFull,
	removeFolderIdentity,
	replaceFolderIdentity,
	setFolderIdentityEnabled,
	setIdentity,
	setSettings,
	setSigning
} from './git-edits.js';
import { emptyGit } from './git-test-support.js';
import { GIT_ALIASES_MAX, GIT_FOLDER_IDENTITIES_MAX } from './git-validation.js';

function rule(folder: string, email = 'a@work.org'): GitFolderIdentity {
	return { folder, name: '', email, signingKey: '', enabled: true };
}

function withFolders(...folders: string[]): GitConfig {
	return { ...emptyGit(3), folderIdentities: folders.map((folder) => rule(folder)) };
}

function withAliases(...names: string[]): GitConfig {
	return {
		...emptyGit(3),
		aliases: names.map((name) => ({ name, command: 'status', enabled: true }))
	};
}

describe('git edits of the values', () => {
	it('replaces the identity, the signature and the settings, keeping the revision', () => {
		let config = setIdentity({ name: 'Ada', email: 'ada@example.org' })(emptyGit(3));
		config = setSigning({ enabled: true, format: 'ssh', key: 'k' })(config);
		config = setSettings({
			defaultBranch: 'main',
			pull: 'rebase',
			editor: 'vim',
			credentialHelper: { kind: 'cache', timeoutSeconds: 900 }
		})(config);
		expect(config.identity.name).toBe('Ada');
		expect(config.signing.format).toBe('ssh');
		expect(config.settings.credentialHelper).toEqual({ kind: 'cache', timeoutSeconds: 900 });
		expect(config.revision).toBe(3);
	});
});

describe('git aliases use the generic alias edits', () => {
	it('add, change, switch and remove', () => {
		const add = addAlias<GitConfig>(
			{ name: 'co', command: 'checkout', enabled: true },
			GIT_ALIASES_MAX
		);
		let config = add(withAliases('st'));
		expect(config.aliases.map(({ name }) => name)).toEqual(['st', 'co']);
		config = replaceAlias<GitConfig>('co', { name: 'ci', command: 'commit', enabled: true })(
			config
		);
		expect(config.aliases.map(({ name }) => name)).toEqual(['st', 'ci']);
		config = setAliasEnabled<GitConfig>('st', false)(config);
		expect(config.aliases[0].enabled).toBe(false);
		config = removeAlias<GitConfig>('st')(config);
		expect(config.aliases.map(({ name }) => name)).toEqual(['ci']);
		expect(config.revision).toBe(3);
	});

	it('stop at the limit of Git, lower than the shell’s', () => {
		const full = withAliases(...Array.from({ length: GIT_ALIASES_MAX }, (_, index) => `a${index}`));
		expect(isFull(full, GIT_ALIASES_MAX)).toBe(true);
		expect(isFull(full)).toBe(false);
		const add = addAlias<GitConfig>(
			{ name: 'more', command: 'status', enabled: true },
			GIT_ALIASES_MAX
		);
		expect(add(full)).toBe(full);
	});
});

describe('edits of the identities by folder', () => {
	it('adds one at the end', () => {
		const config = addFolderIdentity(rule('~/c'))(withFolders('~/a', '~/b'));
		expect(config.folderIdentities.map(({ folder }) => folder)).toEqual(['~/a', '~/b', '~/c']);
		expect(config.revision).toBe(3);
	});

	it('does not add a folder twice', () => {
		const before = withFolders('~/a');
		expect(addFolderIdentity(rule('~/a', 'other@x.org'))(before)).toBe(before);
	});

	it('does not add past the limit', () => {
		const full = withFolders(
			...Array.from({ length: GIT_FOLDER_IDENTITIES_MAX }, (_, index) => `~/f${index}`)
		);
		expect(isFolderListFull(full)).toBe(true);
		expect(addFolderIdentity(rule('~/more'))(full)).toBe(full);
	});

	it('replaces one in place, switches it and removes it', () => {
		let config = replaceFolderIdentity(
			'~/b',
			rule('~/renamed', 'b@x.org')
		)(withFolders('~/a', '~/b'));
		expect(config.folderIdentities.map(({ folder }) => folder)).toEqual(['~/a', '~/renamed']);
		config = setFolderIdentityEnabled('~/a', false)(config);
		expect(config.folderIdentities.map(({ enabled }) => enabled)).toEqual([false, true]);
		config = removeFolderIdentity('~/a')(config);
		expect(config.folderIdentities.map(({ folder }) => folder)).toEqual(['~/renamed']);
	});

	it('changes the settings written by the person', () => {
		const setting = { key: 'push.default', value: 'current', enabled: true };
		const config = editExtraSettings(addSetting(setting))(emptyGit());
		expect(config.extraSettings).toEqual([setting]);
		expect(editExtraSettings(removeSetting('push.default'))(config).extraSettings).toEqual([]);
	});
});
