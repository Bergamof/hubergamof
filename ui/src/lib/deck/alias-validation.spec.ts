import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
	ALIAS_COMMAND_MAX,
	ALIAS_NAME_MAX,
	hasErrors,
	isBlank,
	trimCommand,
	validateAlias
} from './alias-validation.js';

/** The cases the Rust validation (`MachineConfig::validate`) is tested with as well. */
interface SharedCase {
	value: string;
	valid: boolean;
}
const SHARED_CASES: { names: SharedCase[]; commands: SharedCase[] } = JSON.parse(
	readFileSync(
		new URL('../../../../crates/protocol/tests/fixtures/alias-validation.json', import.meta.url),
		'utf8'
	)
);

/** What a case looks like in a failure message: long values and invisible characters made readable. */
function shown(value: string): string {
	return `${JSON.stringify(value).slice(0, 40)} (${[...value].length} code points)`;
}

describe('validateAlias', () => {
	it('accepts a name and a command that follow the rules', () => {
		const errors = validateAlias({ name: '_ll.v2-b', command: 'ls -la' }, ['gs']);
		expect(errors).toEqual({});
		expect(hasErrors(errors)).toBe(false);
	});

	it('asks for a name', () => {
		expect(validateAlias({ name: '', command: 'ls' }, []).name).toMatch(/Saisissez le nom/);
	});

	it.each(['1ll', '-ll', 'l l', 'l;l', 'lé', 'l$'])('refuses the name %j', (name) => {
		expect(validateAlias({ name, command: 'ls' }, []).name).toMatch(/commence par une lettre/);
	});

	it('refuses a name that is too long, and accepts the longest', () => {
		expect(validateAlias({ name: 'a'.repeat(ALIAS_NAME_MAX), command: 'ls' }, [])).toEqual({});
		expect(validateAlias({ name: 'a'.repeat(ALIAS_NAME_MAX + 1), command: 'ls' }, []).name).toMatch(
			/ne dépasse pas 64/
		);
	});

	it('refuses a name already taken, case included', () => {
		expect(validateAlias({ name: 'gs', command: 'git status' }, ['gs']).name).toMatch(
			/porte déjà ce nom/
		);
		expect(validateAlias({ name: 'GS', command: 'git status' }, ['gs'])).toEqual({});
	});

	it('asks for a command, blank ones included', () => {
		expect(validateAlias({ name: 'a', command: '' }, []).command).toMatch(/Saisissez la commande/);
		expect(validateAlias({ name: 'a', command: '   ' }, []).command).toMatch(/Saisissez/);
	});

	it.each(['ls\nrm', 'ls\tla', 'ls\u0007'])('refuses the control characters of %j', (command) => {
		expect(validateAlias({ name: 'a', command }, []).command).toMatch(/une ligne/);
	});

	it('refuses a command that is too long, and accepts the longest', () => {
		expect(validateAlias({ name: 'a', command: 'x'.repeat(ALIAS_COMMAND_MAX) }, [])).toEqual({});
		expect(
			validateAlias({ name: 'a', command: 'x'.repeat(ALIAS_COMMAND_MAX + 1) }, []).command
		).toMatch(/ne dépasse pas 1024/);
	});

	it('reports both fields at once', () => {
		const errors = validateAlias({ name: '', command: '' }, []);
		expect(hasErrors(errors)).toBe(true);
		expect(Object.keys(errors)).toEqual(['name', 'command']);
	});
});

describe('rules shared with the manager', () => {
	it.each(SHARED_CASES.names.map((c) => [shown(c.value), c] as const))(
		'name %s',
		(_label, { value, valid }) => {
			expect(validateAlias({ name: value, command: 'ls' }, []).name === undefined).toBe(valid);
		}
	);

	it.each(SHARED_CASES.commands.map((c) => [shown(c.value), c] as const))(
		'command %s',
		(_label, { value, valid }) => {
			expect(validateAlias({ name: 'll', command: value }, []).command === undefined).toBe(valid);
		}
	);
});

describe('isBlank', () => {
	it.each([
		['empty', ''],
		['spaces', '   '],
		['U+FEFF', '\uFEFF'],
		['U+00A0', '\u00A0'],
		['U+2003', '\u2003'],
		['U+0085', '\u0085'],
		['mixed', ' \uFEFF\u00A0\u2003\t']
	])('holds nothing but white space: %s', (_label, text) => {
		expect(isBlank(text)).toBe(true);
	});

	it.each(['ls', ' ls ', '\uFEFFls', '\u200B'])('is not blank: %j', (text) => {
		expect(isBlank(text)).toBe(false);
	});
});

describe('trimCommand', () => {
	it('keeps the command as typed, minus the white space at its edges', () => {
		expect(trimCommand('  \u00A0ls   -l \u2003\u0085')).toBe('ls   -l');
	});

	it('leaves the inside, a U+FEFF and a command without edges alone', () => {
		expect(trimCommand('ls  -l')).toBe('ls  -l');
		expect(trimCommand(' \uFEFFls')).toBe('\uFEFFls');
	});
});
