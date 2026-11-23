import type { ShellAlias } from '#lib/generated/index.js';

/** Limits of the manager (`ALIAS_NAME_MAX`, `ALIAS_COMMAND_MAX`, `ALIASES_MAX`). */
export const ALIAS_NAME_MAX = 64;
export const ALIAS_COMMAND_MAX = 1024;
export const ALIASES_MAX = 256;

/** A letter or `_`, then letters, digits, `_`, `.` or `-`. */
const ALIAS_NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_.-]*$/;
const CONTROL_CHARACTER = /\p{Cc}/u;

/** What is wrong with the fields of an alias, in French; a field without a problem is absent. */
export interface AliasErrors {
	name?: string;
	command?: string;
}

function lengthOf(text: string): number {
	return [...text].length;
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
	if (command.trim() === '') return 'Saisissez la commande.';
	if (CONTROL_CHARACTER.test(command)) {
		return 'La commande tient sur une ligne, sans caractère de contrôle.';
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
