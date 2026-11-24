import type { ShellAlias } from '#lib/generated/index.js';

/** Limits of the manager (`ALIAS_NAME_MAX`, `ALIAS_COMMAND_MAX`, `ALIASES_MAX`). */
export const ALIAS_NAME_MAX = 64;
export const ALIAS_COMMAND_MAX = 1024;
export const ALIASES_MAX = 256;

/** A letter or `_`, then letters, digits, `_`, `.` or `-`. */
const ALIAS_NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_.-]*$/;
const CONTROL_CHARACTER = /\p{Cc}/u;
/** Characters that show nothing or reorder the text around them (`is_invisible` in `machine.rs`). */
const INVISIBLE_CHARACTER = /[\u200B-\u200F\u2028-\u202E\u2060-\u206F\uFEFF]/u;
/** Unicode white space, as Rust's `char::is_whitespace` knows it (U+0085 included, which `\s` lacks). */
const EDGE_WHITE_SPACE = /^\p{White_Space}+|\p{White_Space}+$/gu;
/** Only white space and the byte order mark (U+FEFF, which shows nothing either). */
const BLANK = /^[\p{White_Space}\uFEFF]*$/u;

/** What is wrong with the fields of an alias, in French; a field without a problem is absent. */
export interface AliasErrors {
	name?: string;
	command?: string;
}

/** Whether a command holds nothing but Unicode white space and U+FEFF. */
export function isBlank(command: string): boolean {
	return BLANK.test(command);
}

/** The command as it is sent: as typed, minus the white space at its edges (a U+FEFF stays). */
export function trimCommand(command: string): string {
	return command.replace(EDGE_WHITE_SPACE, '');
}

/** The length of `text` in Unicode code points, as the manager counts characters. */
export function lengthOf(text: string): number {
	return [...text].length;
}

/** Whether `text` holds a control character or one that shows nothing or reorders the text around it. */
export function hasHiddenCharacter(text: string): boolean {
	return CONTROL_CHARACTER.test(text) || INVISIBLE_CHARACTER.test(text);
}

function nameError(name: string, takenNames: readonly string[]): string | undefined {
	if (name === '') return 'Saisissez le nom de l’alias.';
	if (lengthOf(name) > ALIAS_NAME_MAX) {
		return `Le nom ne dépasse pas ${ALIAS_NAME_MAX} caractères.`;
	}
	if (!ALIAS_NAME_PATTERN.test(name)) {
		return 'Le nom commence par une lettre ou _, puis lettres, chiffres, _, . ou -.';
	}
	if (takenNames.includes(name)) return 'Un alias porte déjà ce nom.';
	return undefined;
}

function commandError(command: string): string | undefined {
	if (isBlank(command)) return 'Saisissez la commande.';
	if (CONTROL_CHARACTER.test(command)) {
		return 'La commande tient sur une ligne, sans caractère de contrôle.';
	}
	if (INVISIBLE_CHARACTER.test(command)) {
		return 'La commande ne contient pas de caractère invisible ni de changement de sens d’écriture.';
	}
	if (lengthOf(command) > ALIAS_COMMAND_MAX) {
		return `La commande ne dépasse pas ${ALIAS_COMMAND_MAX} caractères.`;
	}
	return undefined;
}

/** Mirrors `MachineConfig::validate` for one alias; `takenNames` are the names of the other aliases. */
export function validateAlias(
	alias: Pick<ShellAlias, 'name' | 'command'>,
	takenNames: readonly string[]
): AliasErrors {
	const name = nameError(alias.name, takenNames);
	const command = commandError(alias.command);
	return { ...(name && { name }), ...(command && { command }) };
}

export function hasErrors(errors: AliasErrors): boolean {
	return errors.name !== undefined || errors.command !== undefined;
}
