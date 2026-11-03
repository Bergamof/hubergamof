import type { CatalogModule, ModuleInstanceId } from '#lib/generated/index.js';
import type { DialChoice } from './config-edits.js';

/**
 * Value of the « Aucun » option of the dial module `<select>`; the others are
 * `instance:<id>` (an instance already configured) and `new:<kind>` (a new instance).
 */
export const NO_DIAL_MODULE = 'none';
const EXISTING_PREFIX = 'instance:';
const NEW_PREFIX = 'new:';

export function existingOption(id: ModuleInstanceId): string {
	return `${EXISTING_PREFIX}${id}`;
}

export function newOption(module: CatalogModule): string {
	return `${NEW_PREFIX}${module.kind}`;
}

/** The choice an option stands for; an unknown value means none. */
export function choiceOf(value: string, modules: readonly CatalogModule[]): DialChoice {
	if (value.startsWith(EXISTING_PREFIX)) {
		return { type: 'existing', id: value.slice(EXISTING_PREFIX.length) };
	}
	const module = modules.find((candidate) => newOption(candidate) === value);
	return module ? { type: 'new', module } : { type: 'none' };
}
