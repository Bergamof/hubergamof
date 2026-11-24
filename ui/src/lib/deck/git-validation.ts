import type { GitAlias, GitFolderIdentity } from '#lib/generated/index.js';
import type { AliasErrors } from './alias-validation.js';
import { hasHiddenCharacter, isBlank, lengthOf, trimCommand } from './alias-validation.js';
import {
	ADVANCED_KEY_MAX,
	isOneLineValue,
	valueError,
	type SettingRules
} from './advanced-validation.js';

/** A value as it is sent: as typed, minus the white space at its edges. */
export const trimValue = trimCommand;

/** Limits of the manager (`MAX_GIT_*` and `CREDENTIAL_CACHE_SECONDS` in `git.rs`). */
export const GIT_VALUE_MAX = 256;
/** What a field of text accepts to be typed: twice what is valid, so that the check says why, not the field. */
export const GIT_INPUT_MAX = GIT_VALUE_MAX * 2;
export const GIT_ALIAS_NAME_MAX = 64;
export const GIT_ALIAS_COMMAND_MAX = 1024;
export const GIT_ALIASES_MAX = 128;
export const GIT_FOLDER_IDENTITIES_MAX = 32;
export const CREDENTIAL_CACHE_MIN = 30;
export const CREDENTIAL_CACHE_MAX = 604_800;
export const CREDENTIAL_CACHE_DEFAULT = 900;

/** The editors Git may be told to use by name (`GIT_EDITORS`); any other is an absolute path. */
export const GIT_EDITORS: readonly string[] = [
	'vi',
	'vim',
	'nvim',
	'nano',
	'emacs',
	'micro',
	'hx',
	'code --wait',
	'codium --wait',
	'subl --wait',
	'kate --block',
	'gedit --wait'
];

/** The editors that may be given by the absolute path of their program (`GIT_PLAIN_EDITORS`). */
export const GIT_PLAIN_EDITORS: readonly string[] = [
	'vi',
	'vim',
	'nvim',
	'nano',
	'emacs',
	'micro',
	'hx'
];

const LEADING_WHITE_SPACE = /^\p{White_Space}/u;
const ANY_WHITE_SPACE = /\p{White_Space}/u;
const LEADING_WHITE_SPACE_RUN = /^\p{White_Space}+/u;
const BRANCH_CHARACTERS = /^[A-Za-z0-9._/-]+$/;
const EDITOR_PATH_CHARACTERS = /^[A-Za-z0-9._+/-]+$/;
const ALIAS_NAME_PATTERN = /^[A-Za-z][A-Za-z0-9-]*$/;
const FOLDER_FORBIDDEN = /[*?[\]\\"]/;

function isPlainText(text: string): boolean {
	return lengthOf(text) <= GIT_VALUE_MAX && !hasHiddenCharacter(text);
}

/** `user.name`: on one line, without the `<` and `>` that delimit an address in a commit. */
export function isValidUserName(name: string): boolean {
	return isPlainText(name) && !/[<>]/.test(name) && !LEADING_WHITE_SPACE.test(name);
}

/** `user.email`: one `@` between two non-empty parts, no white space nor `<`, `>`. */
export function isValidUserEmail(email: string): boolean {
	const at = email.indexOf('@');
	if (at < 0) return false;
	const local = email.slice(0, at);
	const domain = email.slice(at + 1);
	return (
		isPlainText(email) &&
		local !== '' &&
		domain !== '' &&
		!domain.includes('@') &&
		!/[<>]/.test(email) &&
		!ANY_WHITE_SPACE.test(email)
	);
}

/** `user.signingkey`: on one line, not an option for the program that signs. */
export function isValidSigningKey(key: string): boolean {
	return isPlainText(key) && !key.startsWith('-') && !LEADING_WHITE_SPACE.test(key);
}

/** `init.defaultBranch`: letters, digits, `.`, `_`, `-` and `/`, as a Git reference name. */
export function isValidBranchName(branch: string): boolean {
	return (
		branch !== '' &&
		lengthOf(branch) <= GIT_VALUE_MAX &&
		BRANCH_CHARACTERS.test(branch) &&
		!/^[-/.]/.test(branch) &&
		!/[/.]$/.test(branch) &&
		!branch.includes('..') &&
		!branch.includes('//') &&
		!branch.includes('/.') &&
		!branch.toLowerCase().endsWith('.lock') &&
		!branch.toLowerCase().includes('.lock/')
	);
}

/** `core.editor`: one of {@link GIT_EDITORS}, or the absolute path of the program of one of {@link GIT_PLAIN_EDITORS}. */
export function isValidEditor(editor: string): boolean {
	if (GIT_EDITORS.includes(editor)) return true;
	return (
		lengthOf(editor) <= GIT_VALUE_MAX &&
		editor.startsWith('/') &&
		editor.length > 1 &&
		GIT_PLAIN_EDITORS.includes(editor.slice(editor.lastIndexOf('/') + 1)) &&
		EDITOR_PATH_CHARACTERS.test(editor) &&
		editor
			.split('/')
			.slice(1)
			.every((part) => part !== '' && part !== '.' && part !== '..')
	);
}

/** The name of a Git alias: a letter, then letters, digits or `-`. */
export function isValidAliasName(name: string): boolean {
	return ALIAS_NAME_PATTERN.test(name) && lengthOf(name) <= GIT_ALIAS_NAME_MAX;
}

/** The command of a Git alias: one line, a Git command (neither `!` nor an option for Git itself). */
export function isValidAliasCommand(command: string): boolean {
	const trimmed = command.replace(LEADING_WHITE_SPACE_RUN, '');
	return (
		!isBlank(trimmed) &&
		lengthOf(trimmed) <= GIT_ALIAS_COMMAND_MAX &&
		!hasHiddenCharacter(trimmed) &&
		!/^[!-]/.test(trimmed)
	);
}

/** The folder of an identity: `~/`, then names (no `.` nor `..`) joined by `/`, no pattern character. */
export function isValidFolder(folder: string): boolean {
	if (!folder.startsWith('~/')) return false;
	const path = folder.slice(2);
	return (
		isPlainText(folder) &&
		path !== '' &&
		!FOLDER_FORBIDDEN.test(path) &&
		path.split('/').every((part) => part !== '' && part !== '.' && part !== '..')
	);
}

/** What is wrong with a value that may be left empty (and then is not set), in French; `undefined` if nothing. */
export function userNameError(name: string): string | undefined {
	if (name === '' || isValidUserName(name)) return undefined;
	return 'Le nom tient sur une ligne, sans « < » ni « > », et ne commence pas par une espace.';
}

export function userEmailError(email: string): string | undefined {
	if (email === '' || isValidUserEmail(email)) return undefined;
	return 'Saisissez une adresse du type nom@domaine, sans espace ni « < » « > ».';
}

export function signingKeyError(key: string): string | undefined {
	if (key === '' || isValidSigningKey(key)) return undefined;
	return 'La clé tient sur une ligne, sans caractère invisible, et ne commence ni par une espace ni par « - ».';
}

export function branchError(branch: string): string | undefined {
	if (branch === '' || isValidBranchName(branch)) return undefined;
	return 'Un nom de branche valide : lettres, chiffres, « . », « _ », « - » et « / », sans « .. », ni « // », ni fin en « / », « . » ou « .lock ».';
}

export function editorError(editor: string): string | undefined {
	if (editor === '' || isValidEditor(editor)) return undefined;
	return 'Choisissez un éditeur de la liste, ou saisissez le chemin absolu du programme de vi, vim, nvim, nano, emacs, micro ou hx (sans espace).';
}

export function cacheTimeoutError(seconds: number): string | undefined {
	return Number.isInteger(seconds) &&
		seconds >= CREDENTIAL_CACHE_MIN &&
		seconds <= CREDENTIAL_CACHE_MAX
		? undefined
		: `La durée est un nombre entier de secondes, de ${CREDENTIAL_CACHE_MIN} à ${CREDENTIAL_CACHE_MAX}.`;
}

/** The names are compared without regard to case: Git does. */
export function validateGitAlias(
	alias: Pick<GitAlias, 'name' | 'command'>,
	takenNames: readonly string[]
): AliasErrors {
	let name: string | undefined;
	if (alias.name === '') name = 'Saisissez le nom de l’alias.';
	else if (lengthOf(alias.name) > GIT_ALIAS_NAME_MAX)
		name = `Le nom ne dépasse pas ${GIT_ALIAS_NAME_MAX} caractères.`;
	else if (!ALIAS_NAME_PATTERN.test(alias.name))
		name = 'Le nom commence par une lettre, puis lettres, chiffres ou -.';
	else if (takenNames.some((taken) => taken.toLowerCase() === alias.name.toLowerCase()))
		name = 'Un alias porte déjà ce nom (Git ne distingue pas les majuscules).';

	let command: string | undefined;
	const trimmed = alias.command.replace(LEADING_WHITE_SPACE_RUN, '');
	if (isBlank(trimmed)) command = 'Saisissez la commande.';
	else if (/^[!-]/.test(trimmed))
		command =
			'Un alias Git est une commande Git : elle ne commence ni par « ! » (commande du shell) ni par « - » (option de Git).';
	else if (hasHiddenCharacter(trimmed))
		command = 'La commande tient sur une ligne, sans caractère de contrôle ni caractère invisible.';
	else if (lengthOf(trimmed) > GIT_ALIAS_COMMAND_MAX)
		command = `La commande ne dépasse pas ${GIT_ALIAS_COMMAND_MAX} caractères.`;
	return { ...(name && { name }), ...(command && { command }) };
}

/** What is wrong with a folder: it must be under the home folder, written `~/…`, and be new. */
export function folderError(folder: string, takenFolders: readonly string[]): string | undefined {
	if (folder === '') return 'Saisissez le dossier.';
	if (!isValidFolder(folder))
		return 'Le dossier est sous votre dossier personnel, écrit « ~/chemin » : sans « / » final, ni « . » ou « .. », ni « * ? [ ] \\ " ».';
	if (takenFolders.includes(folder)) return 'Une identité existe déjà pour ce dossier.';
	return undefined;
}

/** What is wrong with the values of an identity by folder; none can be missing at once. */
export interface FolderIdentityErrors {
	folder?: string;
	name?: string;
	email?: string;
	signingKey?: string;
	/** Nothing is set: the rule would do nothing. */
	empty?: string;
}

export function validateFolderIdentity(
	rule: Pick<GitFolderIdentity, 'folder' | 'name' | 'email' | 'signingKey'>,
	takenFolders: readonly string[]
): FolderIdentityErrors {
	const found: FolderIdentityErrors = {
		folder: folderError(rule.folder, takenFolders),
		name: userNameError(rule.name),
		email: userEmailError(rule.email),
		signingKey: signingKeyError(rule.signingKey)
	};
	if (rule.name === '' && rule.email === '' && rule.signingKey === '') {
		found.empty = 'Renseignez au moins un nom, une adresse ou une clé de signature.';
	}
	return Object.fromEntries(
		Object.entries(found).filter(([, message]) => message !== undefined)
	) as FolderIdentityErrors;
}

export function hasFolderIdentityErrors(errors: FolderIdentityErrors): boolean {
	return Object.keys(errors).length > 0;
}

/** The keys of Git that the forms manage (`GIT_MANAGED_KEYS`), in lower case. */
const MANAGED_KEYS: readonly string[] = [
	'user.name',
	'user.email',
	'user.signingkey',
	'commit.gpgsign',
	'gpg.format',
	'init.defaultbranch',
	'pull.rebase',
	'pull.ff',
	'core.editor',
	'credential.helper'
];

/** The sections whose keys have their own list, or that Git reads as an include. */
const RESERVED_SECTIONS: readonly string[] = ['alias', 'include', 'includeif'];

const KEY_WORD = /^[A-Za-z][A-Za-z0-9-]*$/;
/** The longest sub-section of a key (`MAX_GIT_SUBSECTION_CHARS`). */
export const GIT_SUBSECTION_MAX = 128;
const SUBSECTION_CHARACTERS = /^[A-Za-z0-9._/:@*~+-]+$/;

/** `section`, `subsection` (`null` for none) and `name` of a key: the sub-section is between the first and the last `.`. */
function partsOfKey(key: string): [string, string | null, string] | undefined {
	const first = key.indexOf('.');
	if (first < 0) return undefined;
	const rest = key.slice(first + 1);
	const last = rest.lastIndexOf('.');
	return last < 0
		? [key.slice(0, first), null, rest]
		: [key.slice(0, first), rest.slice(0, last), rest.slice(last + 1)];
}

/** A key as Git compares it (`fold_git_key`): section and name without regard to case, the sub-section as it is. */
export function foldKey(key: string): string {
	const parts = partsOfKey(key);
	if (!parts) return key;
	const [section, subsection, name] = parts;
	return [
		section.toLowerCase(),
		...(subsection === null ? [] : [subsection]),
		name.toLowerCase()
	].join('.');
}

function isKeyWord(word: string): boolean {
	return KEY_WORD.test(word) && lengthOf(word) <= ADVANCED_KEY_MAX;
}

/** Mirrors `is_valid_git_setting_key`: `section.name` or `section.subsection.name` in closed characters, which no form manages. */
export function isValidSettingKey(key: string): boolean {
	const parts = partsOfKey(key);
	if (!parts) return false;
	const [section, subsection, name] = parts;
	const subsectionOk =
		subsection === null ||
		(lengthOf(subsection) <= GIT_SUBSECTION_MAX && SUBSECTION_CHARACTERS.test(subsection));
	return (
		isKeyWord(section) &&
		isKeyWord(name) &&
		subsectionOk &&
		!MANAGED_KEYS.includes(foldKey(key)) &&
		!RESERVED_SECTIONS.includes(section.toLowerCase())
	);
}

/** Mirrors `is_valid_git_setting_value`: one line. */
export function isValidSettingValue(value: string): boolean {
	return isOneLineValue(value);
}

function settingKeyError(key: string): string | undefined {
	if (key === '') return 'Saisissez la clé.';
	if (isValidSettingKey(key)) return undefined;
	const parts = partsOfKey(key);
	if (parts && RESERVED_SECTIONS.includes(parts[0].toLowerCase())) {
		return 'Les alias et les inclusions ont leur propre place : ils ne sont pas acceptés ici.';
	}
	if (MANAGED_KEYS.includes(foldKey(key))) {
		return 'Cette clé a son propre champ dans les formulaires ci-dessus.';
	}
	return 'Une clé s’écrit section.nom (push.default) ou section.sous-section.nom, en lettres, chiffres et « - ».';
}

/** What the settings written by the person must follow: the keys of Git, compared as Git does. */
export const GIT_SETTING_RULES: SettingRules = {
	keyError: settingKeyError,
	valueError: (value) => valueError(value, false),
	fold: foldKey
};
