import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { SshSystemHost } from '#lib/generated/index.js';
import {
	BLANK_HOST,
	draftOf,
	hasHostErrors,
	hostOf,
	isValidAlias,
	isValidHostName,
	isValidIdentityFile,
	isValidProxyJump,
	isValidUser,
	takeOverObstacle,
	validateHost
} from './ssh-validation.js';

interface SharedCase {
	value: string;
	valid: boolean;
}
interface SharedCases {
	aliases: SharedCase[];
	users: SharedCase[];
	hostNames: SharedCase[];
	identityFiles: SharedCase[];
	proxyJumps: SharedCase[];
}

/** The cases the Rust validation (`SshConfig::validate`) is tested with as well. */
const SHARED_CASES: SharedCases = JSON.parse(
	readFileSync(
		new URL('../../../../crates/protocol/tests/fixtures/ssh-validation.json', import.meta.url),
		'utf8'
	)
);

function shown(value: string): string {
	return `${JSON.stringify(value).slice(0, 40)} (${[...value].length} code points)`;
}

describe('the cases shared with the manager', () => {
	const checks: [keyof SharedCases, (value: string) => boolean][] = [
		['aliases', isValidAlias],
		['users', isValidUser],
		['hostNames', isValidHostName],
		['identityFiles', isValidIdentityFile],
		['proxyJumps', isValidProxyJump]
	];
	for (const [group, check] of checks) {
		it(`gives the verdicts of the manager for ${group}`, () => {
			for (const { value, valid } of SHARED_CASES[group]) {
				expect(check(value), shown(value)).toBe(valid);
			}
		});
	}
});

describe('validateHost', () => {
	const draft = (change: Partial<typeof BLANK_HOST>) => ({
		...BLANK_HOST,
		alias: 'prod',
		...change
	});

	it('accepts a host that sets nothing but its name', () => {
		expect(validateHost(draft({}), [])).toEqual({});
	});

	it('says what is wrong with each field', () => {
		const errors = validateHost(
			draft({
				alias: 'a b',
				hostName: 'x;y',
				user: '-u',
				port: '0',
				identityFile: '/etc/passwd',
				proxyJump: '-oProxyCommand=x'
			}),
			[]
		);
		expect(Object.keys(errors).sort()).toEqual(
			['alias', 'hostName', 'identityFile', 'port', 'proxyJump', 'user'].sort()
		);
		expect(hasHostErrors(errors)).toBe(true);
	});

	it('asks for a name and refuses one that is taken', () => {
		expect(validateHost(draft({ alias: '' }), []).alias).toMatch(/Saisissez/);
		expect(validateHost(draft({}), ['prod']).alias).toMatch(/porte déjà ce nom/);
		expect(validateHost(draft({}), ['other']).alias).toBeUndefined();
	});

	it('checks the port as an integer from 1 to 65535', () => {
		for (const port of ['1', '22', '65535', '+22']) {
			expect(validateHost(draft({ port }), []).port, port).toBeUndefined();
		}
		for (const port of ['0', '65536', 'ssh', '2.5', '-1', ' 22']) {
			expect(validateHost(draft({ port }), []).port, port).toBeDefined();
		}
	});
});

describe('the draft of a host', () => {
	it('shows the port as text and gives it back as a number', () => {
		const host = {
			alias: 'a',
			hostName: '',
			user: '',
			port: 2222,
			identityFile: '',
			proxyJump: '',
			forwardAgent: false,
			enabled: true
		};
		expect(draftOf(host).port).toBe('2222');
		expect(hostOf(draftOf(host))).toEqual(host);
		expect(hostOf({ ...BLANK_HOST, alias: 'a' }).port).toBeNull();
		expect(draftOf({ ...host, port: null }).port).toBe('');
	});
});

describe('takeOverObstacle', () => {
	const found = (change: Partial<SshSystemHost>): SshSystemHost => ({
		alias: 'web',
		hostName: 'web.example.org',
		user: 'ada',
		port: null,
		identityFile: '',
		proxyJump: '',
		forwardAgent: false,
		otherOptions: [],
		...change
	});

	it('lets a host with managed options only be taken over', () => {
		expect(takeOverObstacle(found({}), [])).toBeUndefined();
	});

	it('names the options that would be lost', () => {
		expect(takeOverObstacle(found({ otherOptions: ['proxycommand', 'compression'] }), [])).toMatch(
			/proxycommand, compression/
		);
	});

	it('refuses a host whose values do not follow the rules', () => {
		expect(takeOverObstacle(found({ identityFile: '/home/ada/key' }), [])).toMatch(
			/ne respecte pas les règles/
		);
		expect(takeOverObstacle(found({ alias: 'a b' }), [])).toMatch(/ne respecte pas les règles/);
	});
});
