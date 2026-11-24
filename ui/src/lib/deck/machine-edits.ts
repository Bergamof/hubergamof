import type { MachineConfig, ShellAlias } from '#lib/generated/index.js';
import { ALIASES_MAX } from './alias-validation.js';

/** A change of the configuration of the machine. */
export type MachineEdit = (config: MachineConfig) => MachineConfig;

/** A configuration that holds aliases: the machine's, or Git's (an alias has the same fields in both). */
export interface WithAliases {
	aliases: ShellAlias[];
}

/** Whether the configuration holds as many aliases as the manager accepts. */
export function isFull(config: WithAliases, limit: number = ALIASES_MAX): boolean {
	return config.aliases.length >= limit;
}

/** Adds `alias` at the end; no-op when the name is taken or the list is full. */
export function addAlias<C extends WithAliases>(
	alias: ShellAlias,
	limit: number = ALIASES_MAX
): (config: C) => C {
	return (config) =>
		isFull(config, limit) || config.aliases.some(({ name }) => name === alias.name)
			? config
			: { ...config, aliases: [...config.aliases, alias] };
}

/** Replaces the alias called `name` by `alias`, which keeps its place. */
export function replaceAlias<C extends WithAliases>(
	name: string,
	alias: ShellAlias
): (config: C) => C {
	return (config) => ({
		...config,
		aliases: config.aliases.map((existing) => (existing.name === name ? alias : existing))
	});
}

export function setAliasEnabled<C extends WithAliases>(
	name: string,
	enabled: boolean
): (config: C) => C {
	return (config) => ({
		...config,
		aliases: config.aliases.map((alias) => (alias.name === name ? { ...alias, enabled } : alias))
	});
}

export function removeAlias<C extends WithAliases>(name: string): (config: C) => C {
	return (config) => ({
		...config,
		aliases: config.aliases.filter((alias) => alias.name !== name)
	});
}
