import { describe, expect, it } from 'vitest';
import type { SshConfig, SshHost } from '#lib/generated/index.js';
import {
	addHost,
	editCommonOptions,
	editHostOptions,
	isHostListFull,
	removeHost,
	replaceHost,
	setHostEnabled
} from './ssh-edits.js';
import { addSetting } from './advanced-edits.js';
import { SSH_HOSTS_MAX } from './ssh-validation.js';

function host(alias: string): SshHost {
	return {
		alias,
		hostName: '',
		user: '',
		port: null,
		identityFile: '',
		proxyJump: '',
		forwardAgent: false,
		enabled: true,
		options: []
	};
}

function hosts(...aliases: string[]): SshConfig {
	return { schemaVersion: 1, hosts: aliases.map(host), commonOptions: [], revision: 3 };
}

describe('ssh edits', () => {
	it('adds a host at the end, keeping the revision', () => {
		const config = addHost(host('c'))(hosts('a', 'b'));
		expect(config.hosts.map(({ alias }) => alias)).toEqual(['a', 'b', 'c']);
		expect(config.revision).toBe(3);
	});

	it('does not add an alias twice', () => {
		const before = hosts('a');
		expect(addHost({ ...host('a'), user: 'other' })(before)).toBe(before);
	});

	it('does not add past the limit', () => {
		const full = hosts(...Array.from({ length: SSH_HOSTS_MAX }, (_, index) => `h${index}`));
		expect(isHostListFull(full)).toBe(true);
		expect(addHost(host('more'))(full)).toBe(full);
	});

	it('replaces a host in place, switches it and removes it', () => {
		let config = replaceHost('b', host('renamed'))(hosts('a', 'b'));
		expect(config.hosts.map(({ alias }) => alias)).toEqual(['a', 'renamed']);
		config = setHostEnabled('a', false)(config);
		expect(config.hosts.map(({ enabled }) => enabled)).toEqual([false, true]);
		config = removeHost('a')(config);
		expect(config.hosts.map(({ alias }) => alias)).toEqual(['renamed']);
	});

	it('changes the options of one host and leaves the others alone', () => {
		const option = { key: 'Compression', value: 'yes', enabled: true };
		const config = editHostOptions('b', addSetting(option))(hosts('a', 'b'));
		expect(config.hosts.map(({ options }) => options.length)).toEqual([0, 1]);
		expect(config.hosts[1]?.options[0]).toEqual(option);
	});

	it('changes the options common to every host', () => {
		const option = { key: 'ServerAliveInterval', value: '30', enabled: true };
		const config = editCommonOptions(addSetting(option))(hosts('a'));
		expect(config.commonOptions).toEqual([option]);
		expect(config.hosts).toHaveLength(1);
	});
});
