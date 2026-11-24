import { describe, expect, it } from 'vitest';
import type { MachineConfig } from '#lib/generated/index.js';
import { ALIASES_MAX } from './alias-validation.js';
import { addAlias, isFull, removeAlias, replaceAlias, setAliasEnabled } from './machine-edits.js';

function machine(...names: string[]): MachineConfig {
	return {
		schemaVersion: 1,
		aliases: names.map((name) => ({ name, command: `run ${name}`, enabled: true })),
		revision: 3
	};
}

describe('machine edits', () => {
	it('adds an alias at the end, keeping the revision', () => {
		const config = addAlias({ name: 'c', command: 'x', enabled: false })(machine('a', 'b'));
		expect(config.aliases.map(({ name }) => name)).toEqual(['a', 'b', 'c']);
		expect(config.aliases[2].enabled).toBe(false);
		expect(config.revision).toBe(3);
	});

	it('does not add a name twice', () => {
		const before = machine('a');
		expect(addAlias({ name: 'a', command: 'other', enabled: true })(before)).toBe(before);
	});

	it('does not add past the limit', () => {
		const full = machine(...Array.from({ length: ALIASES_MAX }, (_, index) => `a${index}`));
		expect(isFull(full)).toBe(true);
		expect(addAlias({ name: 'more', command: 'x', enabled: true })(full)).toBe(full);
		expect(isFull(machine('a'))).toBe(false);
	});

	it('replaces an alias where it stands', () => {
		const config = replaceAlias('b', { name: 'B', command: 'y', enabled: false })(
			machine('a', 'b', 'c')
		);
		expect(config.aliases.map(({ name }) => name)).toEqual(['a', 'B', 'c']);
		expect(config.aliases[1]).toEqual({ name: 'B', command: 'y', enabled: false });
	});

	it('switches one alias only', () => {
		const config = setAliasEnabled('b', false)(machine('a', 'b'));
		expect(config.aliases.map(({ enabled }) => enabled)).toEqual([true, false]);
	});

	it('removes an alias', () => {
		expect(removeAlias('a')(machine('a', 'b')).aliases.map(({ name }) => name)).toEqual(['b']);
	});
});
