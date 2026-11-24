import type { ShellAlias } from '#lib/generated/index.js';
import {
	ALIAS_COMMAND_MAX,
	ALIAS_NAME_MAX,
	validateAlias,
	type AliasErrors
} from './alias-validation.js';
import { GIT_ALIAS_COMMAND_MAX, GIT_ALIAS_NAME_MAX, validateGitAlias } from './git-validation.js';

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
