import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { SshSystemHost } from '#lib/generated/index.js';
import { validateSetting } from './advanced-validation.js';
import {
	BLANK_HOST,
	draftOf,
	hasHostErrors,
	hostOf,
	isValidAlias,
	isValidHostName,
	isValidIdentityFile,
	isValidOptionKey,
	isValidOptionValue,
	isValidProxyJump,
	isValidUser,
	SSH_OPTION_RULES,
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
	optionKeys: SharedCase[];
	optionValues: SharedCase[];
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
		['proxyJumps', isValidProxyJump],
		['optionKeys', isValidOptionKey],
		['optionValues', isValidOptionValue]
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

	it('names the jump that is wrong and says how to write an IPv6 address', () => {
		const message = validateHost(draft({ proxyJump: 'a,2001:db8::1' }), []).proxyJump;
		expect(message).toMatch(/« 2001:db8::1 » n’est pas valide/);
		expect(message).toMatch(/entre crochets/);
		expect(validateHost(draft({ proxyJump: 'a,b,c,d,e,f' }), []).proxyJump).toMatch(
			/5 rebonds au plus/
		);
		expect(validateHost(draft({ proxyJump: '[2001:db8::1]:22' }), []).proxyJump).toBeUndefined();
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
			enabled: true,
			options: [{ key: 'Compression', value: 'yes', enabled: true }]
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
		options: [],
		lostOptions: [],
		...change
	});

	it('lets a host with managed options only be taken over', () => {
		expect(takeOverObstacle(found({}), [])).toBeUndefined();
	});

	it('names the options that would be lost', () => {
		const obstacle = takeOverObstacle(
			found({ lostOptions: ['identityfile', 'port', 'compression'] }),
			[]
		);
		expect(obstacle).toMatch(/Ne peut pas être repris sans perte/);
		expect(obstacle).toMatch(/identityfile \(plusieurs clés\)/);
		expect(obstacle).toMatch(/port \(une valeur qui n’est pas un nombre\)/);
		expect(obstacle).toMatch(/compression \(écrite deux fois, ou hors des règles/);
	});

	it('lets a host with options of its own be taken over, and refuses one whose options break the rules', () => {
		const option = { key: 'ProxyCommand', value: 'nc %h %p', enabled: true };
		expect(takeOverObstacle(found({ options: [option] }), [])).toBeUndefined();
		expect(takeOverObstacle(found({ options: [{ ...option, key: 'Host' }] }), [])).toMatch(
			/L’option « Host »/
		);
		expect(
			takeOverObstacle(found({ options: [option, { ...option, key: 'proxycommand' }] }), [])
		).toMatch(/déjà dans la liste/);
	});

	it('refuses a host whose values do not follow the rules', () => {
		expect(takeOverObstacle(found({ identityFile: '/home/ada/key' }), [])).toMatch(
			/Ne respecte pas les règles/
		);
		expect(takeOverObstacle(found({ alias: 'a b' }), [])).toMatch(/Ne respecte pas les règles/);
	});
});

describe('the options written by the person', () => {
	const check = (key: string, value: string, takenKeys: string[] = []) =>
		validateSetting({ key, value }, takenKeys, SSH_OPTION_RULES);

	it('accepts any option that is not managed elsewhere', () => {
		expect(check('ServerAliveInterval', '30')).toEqual({});
		expect(check('ProxyCommand', 'ssh -W %h:%p bastion')).toEqual({});
	});

	it('says why a key is refused', () => {
		expect(check('', 'x').name).toMatch(/nom de l’option/);
		expect(check('Host', 'x').name).toMatch(/autre section/);
		expect(check('match', 'x').name).toMatch(/autre section/);
		expect(check('User', 'x').name).toMatch(/propre champ/);
		expect(check('Server-Alive', 'x').name).toMatch(/commence par une lettre/);
	});

	it('says why a value is refused', () => {
		expect(check('A', '').command).toMatch(/Saisissez la valeur/);
		expect(check('A', 'a\nb').command).toMatch(/une ligne/);
		expect(check('A', '"open').command).toMatch(/guillemets/);
		expect(check('A', 'x'.repeat(1025)).command).toMatch(/1024 caractères/);
	});

	it('compares keys without regard to case', () => {
		expect(check('compression', 'no', ['Compression']).name).toMatch(/déjà dans la liste/);
		expect(check('Compression', 'no', ['Other'])).toEqual({});
	});
});
