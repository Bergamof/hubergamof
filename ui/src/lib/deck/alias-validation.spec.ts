import { describe, expect, it } from 'vitest';
import { ALIAS_COMMAND_MAX, ALIAS_NAME_MAX, hasErrors, validateAlias } from './alias-validation.js';

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
