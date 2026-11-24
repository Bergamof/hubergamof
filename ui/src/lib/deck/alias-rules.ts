import type { ShellAlias } from '#lib/generated/index.js';
import {
	ALIAS_COMMAND_MAX,
	ALIAS_NAME_MAX,
	validateAlias,
	type AliasErrors
} from './alias-validation.js';
import { ADVANCED_KEY_MAX, ADVANCED_VALUE_MAX, validateSetting } from './advanced-validation.js';
import {
	GIT_ALIAS_COMMAND_MAX,
	GIT_ALIAS_NAME_MAX,
	GIT_SETTING_RULES,
	validateGitAlias
} from './git-validation.js';
import { SSH_OPTION_RULES } from './ssh-validation.js';

/** What tells the shell aliases and the Git aliases apart for the forms and lists that handle both. */
export interface AliasRules {
	nameMax: number;
	commandMax: number;
	/** Mirrors the manager's validation of one alias; `takenNames` are the names of the others. */
	validate: (
		alias: Pick<ShellAlias, 'name' | 'command'>,
		takenNames: readonly string[]
	) => AliasErrors;
	/** Why an alias of the system that does not follow the rules cannot be taken in. */
	refusal: string;
	/** What names the kind of alias in a sentence (« l’alias Git »). */
	noun: string;
	/** The same, capitalised, to start a label. */
	title: string;
	/** What the button that takes an alias found elsewhere in says. */
	adoptVerb: string;
	/** Names the list of the aliases found elsewhere. */
	foundLabel: string;
	/** Labels of the two fields of the form; « Nom » and « Commande » by default. */
	nameLabel?: string;
	commandLabel?: string;
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
 * The lists of settings written by the person (an option of `ssh`, a key of Git) are handled by the
 * forms and lists of the aliases: a key stands for the name, a value for the command. There is no
 * « found elsewhere » list for them.
 */
export const SSH_OPTION_ALIAS_RULES: AliasRules = {
	nameMax: ADVANCED_KEY_MAX,
	commandMax: ADVANCED_VALUE_MAX,
	validate: (setting, takenNames) =>
		validateSetting({ key: setting.name, value: setting.command }, takenNames, SSH_OPTION_RULES),
	refusal: '',
	noun: 'option',
	title: 'Option',
	adoptVerb: '',
	foundLabel: '',
	nameLabel: 'Option',
	commandLabel: 'Valeur'
};

export const GIT_SETTING_ALIAS_RULES: AliasRules = {
	nameMax: ADVANCED_KEY_MAX + 2 * ADVANCED_KEY_MAX,
	commandMax: ADVANCED_VALUE_MAX,
	validate: (setting, takenNames) =>
		validateSetting({ key: setting.name, value: setting.command }, takenNames, GIT_SETTING_RULES),
	refusal: '',
	noun: 'option Git',
	title: 'Option Git',
	adoptVerb: '',
	foundLabel: '',
	nameLabel: 'Clé',
	commandLabel: 'Valeur'
};
