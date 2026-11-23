import type { MachineConfig, ShellAlias } from '#lib/generated/index.js';
import { ALIASES_MAX } from './alias-validation.js';

/** A change of the configuration of the machine. */
export type MachineEdit = (config: MachineConfig) => MachineConfig;

/** Whether the configuration holds as many aliases as the manager accepts. */
export function isFull(config: MachineConfig): boolean {
	return config.aliases.length >= ALIASES_MAX;
}

/** Adds `alias` at the end; no-op when the name is taken or the list is full. */
export function addAlias(alias: ShellAlias): MachineEdit {
	return (config) =>
		isFull(config) || config.aliases.some(({ name }) => name === alias.name)
			? config
			: { ...config, aliases: [...config.aliases, alias] };
}

/** Replaces the alias called `name` by `alias`, which keeps its place. */
export function replaceAlias(name: string, alias: ShellAlias): MachineEdit {
	return (config) => ({
		...config,
		aliases: config.aliases.map((existing) => (existing.name === name ? alias : existing))
	});
}

export function setAliasEnabled(name: string, enabled: boolean): MachineEdit {
	return (config) => ({
		...config,
		aliases: config.aliases.map((alias) => (alias.name === name ? { ...alias, enabled } : alias))
	});
}

export function removeAlias(name: string): MachineEdit {
	return (config) => ({
		...config,
		aliases: config.aliases.filter((alias) => alias.name !== name)
	});
}
