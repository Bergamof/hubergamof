import type { AdvancedSetting, ShellAlias } from '#lib/generated/index.js';
import { ADVANCED_MAX } from './advanced-validation.js';

/** A change of a list of settings written by the person. */
export type SettingsEdit = (settings: AdvancedSetting[]) => AdvancedSetting[];

/** Whether the list holds as many settings as the manager accepts. */
export function isSettingsListFull(settings: readonly AdvancedSetting[]): boolean {
	return settings.length >= ADVANCED_MAX;
}

/** Adds `setting` at the end; no-op when its key is taken or the list is full. */
export function addSetting(setting: AdvancedSetting): SettingsEdit {
	return (settings) =>
		isSettingsListFull(settings) || settings.some(({ key }) => key === setting.key)
			? settings
			: [...settings, setting];
}

/** Replaces the setting of `key` by `setting`, which keeps its place. */
export function replaceSetting(key: string, setting: AdvancedSetting): SettingsEdit {
	return (settings) => settings.map((existing) => (existing.key === key ? setting : existing));
}

export function setSettingEnabled(key: string, enabled: boolean): SettingsEdit {
	return (settings) =>
		settings.map((setting) => (setting.key === key ? { ...setting, enabled } : setting));
}

export function removeSetting(key: string): SettingsEdit {
	return (settings) => settings.filter((setting) => setting.key !== key);
}

/** The forms and lists of the aliases handle settings: the key is the name, the value the command. */
export function asAlias({ key, value, enabled }: AdvancedSetting): ShellAlias {
	return { name: key, command: value, enabled };
}

export function fromAlias({ name, command, enabled }: ShellAlias): AdvancedSetting {
	return { key: name, value: command, enabled };
}

/** What a list that holds as many settings as the manager accepts says instead of its form. */
export const SETTINGS_FULL_TEXT =
	'La liste contient déjà le nombre maximal d’éléments : supprimez-en un pour en ajouter.';
