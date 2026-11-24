import type { SshConfig, SshHost } from '#lib/generated/index.js';
import { SSH_HOSTS_MAX } from './ssh-validation.js';

/** A change of the SSH configuration. */
export type SshEdit = (config: SshConfig) => SshConfig;

/** Whether the configuration holds as many hosts as the manager accepts. */
export function isHostListFull(config: SshConfig): boolean {
	return config.hosts.length >= SSH_HOSTS_MAX;
}

/** Adds `host` at the end; no-op when its alias is taken or the list is full. */
export function addHost(host: SshHost): SshEdit {
	return (config) =>
		isHostListFull(config) || config.hosts.some(({ alias }) => alias === host.alias)
			? config
			: { ...config, hosts: [...config.hosts, host] };
}

/** Replaces the host called `alias` by `host`, which keeps its place. */
export function replaceHost(alias: string, host: SshHost): SshEdit {
	return (config) => ({
		...config,
		hosts: config.hosts.map((existing) => (existing.alias === alias ? host : existing))
	});
}

export function setHostEnabled(alias: string, enabled: boolean): SshEdit {
	return (config) => ({
		...config,
		hosts: config.hosts.map((host) => (host.alias === alias ? { ...host, enabled } : host))
	});
}

export function removeHost(alias: string): SshEdit {
	return (config) => ({ ...config, hosts: config.hosts.filter((host) => host.alias !== alias) });
}
