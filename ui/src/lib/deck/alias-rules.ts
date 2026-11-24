import type { ShellAlias } from '#lib/generated/index.js';
import {
	ALIAS_COMMAND_MAX,
	ALIAS_NAME_MAX,
	validateAlias,
	type AliasErrors
} from './alias-validation.js';
import {
	ADVANCED_KEY_MAX,
	ADVANCED_VALUE_MAX,
	validateSetting,
	type SettingRules
} from './advanced-validation.js';
import {
	GIT_ALIAS_COMMAND_MAX,
	GIT_ALIAS_NAME_MAX,
	GIT_SETTING_RULES,
	GIT_SUBSECTION_MAX,
	validateGitAlias
} from './git-validation.js';
import { SSH_OPTION_RULES } from './ssh-validation.js';

/**
 * What the forms and lists of aliases need to know of what they hold: the shell aliases, the Git
 * aliases, and the settings written by the person (a key standing for the name, a value for the
 * command).
 */
export interface EntryRules {
	nameMax: number;
	commandMax: number;
	/** Mirrors the manager's validation of one entry; `takenNames` are the names of the others. */
	validate: (
		alias: Pick<ShellAlias, 'name' | 'command'>,
		takenNames: readonly string[]
	) => AliasErrors;
	/** What names the kind of entry in a sentence (« l’alias Git »). */
	noun: string;
	/** The same, capitalised, to start a label. */
	title: string;
	/** Whether the noun is feminine, for the words that agree with it (« gérée », « retirée »). */
	feminine?: boolean;
	/** Labels of the two fields of the form; « Nom » and « Commande » by default. */
	nameLabel?: string;
	commandLabel?: string;
	/** What the empty name field shows as an example. */
	nameHint?: string;
}

/** What the list of the aliases found elsewhere adds. */
export interface AliasRules extends EntryRules {
	/** Why an alias of the system that does not follow the rules cannot be taken in. */
	refusal: string;
	/** What the button that takes an alias found elsewhere in says. */
	adoptVerb: string;
	/** Names the list of the aliases found elsewhere. */
	foundLabel: string;
}

/** `gérés` or `gérées`, as the noun of the rules requires. */
export function managedWord(rules: EntryRules): string {
	return rules.feminine ? 'gérées' : 'gérés';
}

export const SHELL_ALIAS_RULES: AliasRules = {
	nameMax: ALIAS_NAME_MAX,
	commandMax: ALIAS_COMMAND_MAX,
	validate: validateAlias,
	refusal: 'Cet alias ne respecte pas les règles de HuBergamof (nom ou commande).',
	noun: 'alias',
	title: 'Alias',
	adoptVerb: 'Intégrer',
	foundLabel: 'Alias du système, non gérés'
};

export const GIT_ALIAS_RULES: AliasRules = {
	nameMax: GIT_ALIAS_NAME_MAX,
	commandMax: GIT_ALIAS_COMMAND_MAX,
	validate: validateGitAlias,
	refusal:
		'HuBergamof ne gère pas cet alias : son nom n’est pas valide, ou il lance une commande du shell (« ! ») ou une option de Git (« - »).',
	noun: 'alias Git',
	title: 'Alias Git',
	adoptVerb: 'Reprendre',
	foundLabel: 'Alias Git définis ailleurs, non gérés'
};

/**
 * The settings written by the person (an option of `ssh`, a key of Git) are handled by the forms and
 * lists of the aliases: a key stands for the name, a value for the command.
 */
function settingRules(
	rules: SettingRules,
	labels: Pick<EntryRules, 'noun' | 'title' | 'nameLabel' | 'nameHint'>,
	nameMax: number
): EntryRules {
	return {
		...labels,
		nameMax,
		commandMax: ADVANCED_VALUE_MAX,
		feminine: true,
		commandLabel: 'Valeur',
		validate: (setting, takenNames) =>
			validateSetting({ key: setting.name, value: setting.command }, takenNames, rules)
	};
}

export const SSH_OPTION_ALIAS_RULES: EntryRules = settingRules(
	SSH_OPTION_RULES,
	{
		noun: 'option',
		title: 'Option',
		nameLabel: 'Nom de l’option',
		nameHint: 'ServerAliveInterval'
	},
	ADVANCED_KEY_MAX
);

/** The longest key: a section, a sub-section and a name, with their two dots. */
const GIT_KEY_MAX = ADVANCED_KEY_MAX + GIT_SUBSECTION_MAX + ADVANCED_KEY_MAX + 2;

export const GIT_SETTING_ALIAS_RULES: EntryRules = settingRules(
	GIT_SETTING_RULES,
	{ noun: 'option Git', title: 'Option Git', nameLabel: 'Clé Git', nameHint: 'push.default' },
	GIT_KEY_MAX
);
