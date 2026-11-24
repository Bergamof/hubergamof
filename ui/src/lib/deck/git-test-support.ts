import type { GitConfig } from '#lib/generated/index.js';

/** A Git configuration that sets nothing, as the manager creates it. */
export function emptyGit(revision = 0): GitConfig {
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
		revision
	};
}
