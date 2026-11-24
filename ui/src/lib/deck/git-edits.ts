import type {
	GitConfig,
	GitFolderIdentity,
	GitIdentity,
	GitSettings,
	GitSigning
} from '#lib/generated/index.js';
import { GIT_FOLDER_IDENTITIES_MAX } from './git-validation.js';

/** A change of the Git configuration. */
export type GitEdit = (config: GitConfig) => GitConfig;

export function setIdentity(identity: GitIdentity): GitEdit {
	return (config) => ({ ...config, identity });
}

export function setSigning(signing: GitSigning): GitEdit {
	return (config) => ({ ...config, signing });
}

export function setSettings(settings: GitSettings): GitEdit {
	return (config) => ({ ...config, settings });
}

/** Whether the configuration holds as many identities by folder as the manager accepts. */
export function isFolderListFull(config: GitConfig): boolean {
	return config.folderIdentities.length >= GIT_FOLDER_IDENTITIES_MAX;
}

/** Adds `rule` at the end; no-op when its folder has one already or the list is full. */
export function addFolderIdentity(rule: GitFolderIdentity): GitEdit {
	return (config) =>
		isFolderListFull(config) || config.folderIdentities.some(({ folder }) => folder === rule.folder)
			? config
			: { ...config, folderIdentities: [...config.folderIdentities, rule] };
}

/** Replaces the identity of `folder` by `rule`, which keeps its place. */
export function replaceFolderIdentity(folder: string, rule: GitFolderIdentity): GitEdit {
	return (config) => ({
		...config,
		folderIdentities: config.folderIdentities.map((existing) =>
			existing.folder === folder ? rule : existing
		)
	});
}

export function setFolderIdentityEnabled(folder: string, enabled: boolean): GitEdit {
	return (config) => ({
		...config,
		folderIdentities: config.folderIdentities.map((rule) =>
			rule.folder === folder ? { ...rule, enabled } : rule
		)
	});
}

export function removeFolderIdentity(folder: string): GitEdit {
	return (config) => ({
		...config,
		folderIdentities: config.folderIdentities.filter((rule) => rule.folder !== folder)
	});
}
