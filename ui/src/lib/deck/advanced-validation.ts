import type { AdvancedSetting } from '#lib/generated/index.js';
import type { AliasErrors } from './alias-validation.js';
import { hasHiddenCharacter, lengthOf } from './alias-validation.js';

/** Limits of the manager (`MAX_ADVANCED_SETTINGS`, `MAX_SETTING_*_CHARS` in `advanced.rs`). */
export const ADVANCED_MAX = 64;
export const ADVANCED_KEY_MAX = 64;
export const ADVANCED_VALUE_MAX = 1024;

const EDGE_WHITE_SPACE = /^\p{White_Space}|\p{White_Space}$/u;

/** Mirrors `is_one_line_value`: not empty, no blank at its edges, no control or invisible character. */
export function isOneLineValue(value: string): boolean {
	return (
		value !== '' &&
		lengthOf(value) <= ADVANCED_VALUE_MAX &&
		!EDGE_WHITE_SPACE.test(value) &&
		!hasHiddenCharacter(value)
	);
}

/** Mirrors `has_paired_quotes`: `"` and `'` each an even number of times. */
export function hasPairedQuotes(value: string): boolean {
	const count = (quote: string) => value.split(quote).length - 1;
	return count('"') % 2 === 0 && count("'") % 2 === 0;
}

/** What is wrong with the value of a setting, in French; `undefined` when it is fine. */
export function valueError(value: string, pairedQuotes: boolean): string | undefined {
	if (value === '') return 'Saisissez la valeur.';
	if (hasHiddenCharacter(value)) {
		return 'La valeur tient sur une ligne, sans caractère de contrôle ni caractère invisible.';
	}
	if (lengthOf(value) > ADVANCED_VALUE_MAX) {
		return `La valeur ne dépasse pas ${ADVANCED_VALUE_MAX} caractères.`;
	}
	if (pairedQuotes && !hasPairedQuotes(value)) {
		return 'Les guillemets (" et \') vont par deux : ssh refuserait toute la configuration.';
	}
	return undefined;
}

/**
 * What the form of a list of settings needs to check one of them: the rules of the file it is
 * written to.
 */
export interface SettingRules {
	/** Whether the key can be written (`is_valid_*_key`). */
	keyError: (key: string) => string | undefined;
	/** Whether the value can be written. */
	valueError: (value: string) => string | undefined;
	/** The key as the program compares it: two keys that fold to the same are the same. */
	fold: (key: string) => string;
}

/**
 * Mirrors the check of a list of settings for one of them; `takenKeys` are the keys of the others.
 * The fields are called `name` and `command` for the forms shared with the aliases.
 */
export function validateSetting(
	setting: Pick<AdvancedSetting, 'key' | 'value'>,
	takenKeys: readonly string[],
	rules: SettingRules
): AliasErrors {
	const name =
		rules.keyError(setting.key) ??
		(takenKeys.some((taken) => rules.fold(taken) === rules.fold(setting.key))
			? 'Cette clé est déjà dans la liste : une clé n’y figure qu’une fois.'
			: undefined);
	const command = rules.valueError(setting.value);
	return { ...(name && { name }), ...(command && { command }) };
}
