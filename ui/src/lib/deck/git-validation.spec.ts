import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { validateSetting } from './advanced-validation.js';
import { hasErrors } from './alias-validation.js';
import {
	CREDENTIAL_CACHE_MAX,
	CREDENTIAL_CACHE_MIN,
	GIT_EDITORS,
	GIT_PLAIN_EDITORS,
	branchError,
	cacheTimeoutError,
	editorError,
	folderError,
	hasFolderIdentityErrors,
	isValidAliasCommand,
	isValidAliasName,
	isValidBranchName,
	isValidEditor,
	isValidFolder,
	isValidSettingKey,
	isValidSettingValue,
	isValidSigningKey,
	isValidUserEmail,
	isValidUserName,
	signingKeyError,
	userEmailError,
	userNameError,
	GIT_SETTING_RULES,
	foldKey,
	validateFolderIdentity,
	validateGitAlias
} from './git-validation.js';

interface SharedCase {
	value: string;
	valid: boolean;
}
interface SharedCases {
	names: SharedCase[];
	emails: SharedCase[];
	keys: SharedCase[];
	branches: SharedCase[];
	editors: SharedCase[];
	knownEditors: string[];
	plainEditors: string[];
	aliasNames: SharedCase[];
	aliasCommands: SharedCase[];
	folders: SharedCase[];
	settingKeys: SharedCase[];
	settingValues: SharedCase[];
}

/** The cases the Rust validation (`GitConfig::validate`) is tested with as well. */
const SHARED_CASES: SharedCases = JSON.parse(
	readFileSync(
		new URL('../../../../crates/protocol/tests/fixtures/git-validation.json', import.meta.url),
		'utf8'
	)
);

function shown(value: string): string {
	return `${JSON.stringify(value).slice(0, 40)} (${[...value].length} code points)`;
}

describe('the cases shared with the manager', () => {
	const checks: [keyof SharedCases, (value: string) => boolean][] = [
		['names', isValidUserName],
		['emails', isValidUserEmail],
		['keys', isValidSigningKey],
		['branches', isValidBranchName],
		['editors', isValidEditor],
		['aliasNames', isValidAliasName],
		['aliasCommands', isValidAliasCommand],
		['folders', isValidFolder],
		['settingKeys', isValidSettingKey],
		['settingValues', isValidSettingValue]
	];
	for (const [group, check] of checks) {
		it(`gives the verdicts of the manager for ${group}`, () => {
			for (const { value, valid } of SHARED_CASES[group] as SharedCase[]) {
				expect(check(value), shown(value)).toBe(valid);
			}
		});
	}

	it('lists the editors the manager knows', () => {
		expect([...GIT_EDITORS]).toEqual(SHARED_CASES.knownEditors);
		expect([...GIT_PLAIN_EDITORS]).toEqual(SHARED_CASES.plainEditors);
	});
});

describe('the messages of the fields', () => {
	it('leave an empty optional field alone', () => {
		for (const error of [
			userNameError,
			userEmailError,
			signingKeyError,
			branchError,
			editorError
		]) {
			expect(error('')).toBeUndefined();
		}
	});

	it('say what is wrong with a value', () => {
		expect(userNameError('Ada <x>')).toMatch(/sans « < »/);
		expect(userEmailError('ada')).toMatch(/nom@domaine/);
		expect(signingKeyError('-u')).toMatch(/ne commence ni par une espace ni par « - »/);
		expect(branchError('a..b')).toMatch(/nom de branche valide/);
		expect(editorError('vim --x')).toMatch(/chemin absolu/);
	});

	it('check the cache duration within its range', () => {
		expect(cacheTimeoutError(CREDENTIAL_CACHE_MIN)).toBeUndefined();
		expect(cacheTimeoutError(CREDENTIAL_CACHE_MAX)).toBeUndefined();
		expect(cacheTimeoutError(CREDENTIAL_CACHE_MIN - 1)).toMatch(/de 30 à 604800/);
		expect(cacheTimeoutError(CREDENTIAL_CACHE_MAX + 1)).toBeDefined();
		expect(cacheTimeoutError(60.5)).toBeDefined();
	});
});

describe('validateGitAlias', () => {
	it('accepts a Git command', () => {
		expect(validateGitAlias({ name: 'co', command: 'checkout' }, ['st'])).toEqual({});
	});

	it('refuses a shell command and an option of Git itself, saying why', () => {
		for (const command of ['!rm -rf ~', '  !ls', '-c core.pager=evil log']) {
			const errors = validateGitAlias({ name: 'x', command }, []);
			expect(errors.command, command).toMatch(/ne commence ni par « ! »/);
			expect(hasErrors(errors)).toBe(true);
		}
	});

	it('compares names without regard to case', () => {
		expect(validateGitAlias({ name: 'CO', command: 'checkout' }, ['co']).name).toMatch(
			/Un alias porte déjà ce nom/
		);
	});

	it('refuses names that Git would not accept as a key', () => {
		for (const name of ['a_b', 'a.b', '1a', '-a', 'é', 'a b']) {
			expect(validateGitAlias({ name, command: 'status' }, []).name, name).toMatch(
				/commence par une lettre/
			);
		}
		expect(validateGitAlias({ name: '', command: 'status' }, []).name).toMatch(/Saisissez/);
		expect(validateGitAlias({ name: 'a'.repeat(65), command: 'status' }, []).name).toMatch(
			/ne dépasse pas 64/
		);
	});

	it('refuses an empty, multi-line or oversized command', () => {
		expect(validateGitAlias({ name: 'x', command: '   ' }, []).command).toMatch(/Saisissez/);
		expect(validateGitAlias({ name: 'x', command: 'a\nb' }, []).command).toMatch(/une ligne/);
		expect(validateGitAlias({ name: 'x', command: 'c'.repeat(1025) }, []).command).toMatch(
			/ne dépasse pas 1024/
		);
	});
});

describe('folders and their identities', () => {
	it('checks the folder', () => {
		expect(folderError('', [])).toMatch(/Saisissez/);
		expect(folderError('/etc', [])).toMatch(/« ~\/chemin »/);
		expect(folderError('~/work', ['~/work'])).toMatch(/existe déjà/);
		expect(folderError('~/work', ['~/other'])).toBeUndefined();
	});

	it('wants something set, and every value right', () => {
		const empty = validateFolderIdentity(
			{ folder: '~/w', name: '', email: '', signingKey: '' },
			[]
		);
		expect(empty.empty).toMatch(/au moins/);
		expect(hasFolderIdentityErrors(empty)).toBe(true);

		const wrong = validateFolderIdentity(
			{ folder: '~/w', name: '', email: 'nope', signingKey: '' },
			[]
		);
		expect(Object.keys(wrong)).toEqual(['email']);

		const fine = validateFolderIdentity(
			{ folder: '~/w', name: '', email: 'a@work.org', signingKey: '' },
			[]
		);
		expect(hasFolderIdentityErrors(fine)).toBe(false);
	});
});

describe('the settings written by the person', () => {
	const check = (key: string, value: string, takenKeys: string[] = []) =>
		validateSetting({ key, value }, takenKeys, GIT_SETTING_RULES);

	it('accepts a key of Git that no form manages', () => {
		expect(check('push.default', 'current')).toEqual({});
		expect(check('url.https://example.org/.insteadOf', 'ex:')).toEqual({});
	});

	it('says why a key is refused', () => {
		expect(check('', 'x').name).toMatch(/Saisissez la clé/);
		expect(check('alias.co', 'x').name).toMatch(/propre place/);
		expect(check('includeIf.gitdir:~/x/.path', 'x').name).toMatch(/propre place/);
		expect(check('USER.Email', 'x').name).toMatch(/propre champ/);
		expect(check('push', 'x').name).toMatch(/section\.nom/);
	});

	it('says why a value is refused', () => {
		expect(check('a.b', '').command).toMatch(/Saisissez la valeur/);
		expect(check('a.b', ' x').command).toBeUndefined();
	});

	it('compares keys as Git does: section and name without case, sub-section as it is', () => {
		expect(foldKey('Core.AutoCRLF')).toBe('core.autocrlf');
		expect(foldKey('URL.Https://X/.InsteadOf')).toBe('url.Https://X/.insteadof');
		expect(check('Push.Default', 'x', ['push.default']).name).toMatch(/déjà dans la liste/);
		expect(check('url.A.insteadOf', 'x', ['url.a.insteadOf'])).toEqual({});
	});
});
