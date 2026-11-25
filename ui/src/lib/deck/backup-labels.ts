import type { BackupGroup } from '#lib/api/manager-api.js';
import type { ExecutableKind, SectionChange, SectionResult } from '#lib/generated/index.js';

/** What the user ticks to export, in the order of the sidebar. */
export const GROUP_LABELS: Record<BackupGroup, string> = {
	devices: 'Stream Deck',
	mice: 'Souris',
	'radial-menu': 'Menu circulaire',
	modules: 'Modules',
	machine: 'Alias du shell',
	git: 'Git',
	ssh: 'SSH'
};

const FIXED_SECTIONS: Record<string, string> = {
	'radial-menu': GROUP_LABELS['radial-menu'],
	modules: GROUP_LABELS.modules,
	machine: GROUP_LABELS.machine,
	git: GROUP_LABELS.git,
	ssh: GROUP_LABELS.ssh
};

/**
 * The name of a section of a backup (`devices/<serial>`, `mice/<serial>`, `git`…) as shown to the
 * user; a device or mouse that is connected is shown by the name `names` gives its serial.
 */
export function sectionLabel(id: string, names: ReadonlyMap<string, string> = new Map()): string {
	const fixed = FIXED_SECTIONS[id];
	if (fixed) return fixed;
	const [kind, serial] = id.split('/', 2);
	const name = names.get(serial);
	if (kind === 'devices') return name ? `${name} (${serial})` : `Stream Deck ${serial}`;
	if (kind === 'mice') return name ? `${name} (${serial})` : `Souris ${serial}`;
	return id;
}

/** Serial of the device a section holds, `undefined` for any other section. */
export function deviceSerialOf(id: string): string | undefined {
	return id.startsWith('devices/') ? id.slice('devices/'.length) : undefined;
}

/** What an import would do to a section, in words (never by colour alone). */
export const CHANGE_LABELS: Record<SectionChange, string> = {
	added: 'Ajoutée',
	replaced: 'Remplacée',
	same: 'Identique'
};

export const EXECUTABLE_LABELS: Record<ExecutableKind, string> = {
	shellAlias: 'Alias du shell',
	gitAlias: 'Alias Git',
	gitSetting: 'Réglage Git',
	sshHost: 'Hôte SSH',
	sshOption: 'Option SSH'
};

/** What became of a section: the sentence, and the manager's own words (English) when it gave some. */
export function outcomeText(result: SectionResult): { label: string; detail?: string } {
	switch (result.result) {
		case 'applied':
			return { label: 'Appliquée' };
		case 'stored':
			return { label: 'Gardée pour son prochain branchement' };
		case 'notApplied':
			return {
				label: 'Enregistrée, mais pas appliquée à l’ordinateur',
				detail: result.message
			};
		case 'refused':
			return { label: 'Refusée, laissée telle quelle', detail: result.message };
	}
}

/** `seconds` since the Unix epoch, as a French date and time. */
export function snapshotDate(seconds: number): string {
	return new Intl.DateTimeFormat('fr-FR', { dateStyle: 'long', timeStyle: 'short' }).format(
		new Date(seconds * 1000)
	);
}
