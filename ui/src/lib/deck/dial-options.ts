import type {
	CatalogModule,
	DialAssignment,
	DialFunctionDescriptor,
	ModuleInstanceId
} from '#lib/generated/index.js';
import type { DialChoice } from './config-edits.js';

/**
 * Values of the options of the dial function `<select>`: the functions of the manager, then
 * `function:<instance>/<function>` (a module already configured) and `new:<kind>/<function>`
 * (a new instance of a module). Identifiers hold no `/`.
 */
export const NO_DIAL_FUNCTION = 'none';
export const NAVIGATION_OPTION = 'navigation';
export const CONTEXTUAL_OPTION = 'contextual';
const EXISTING_PREFIX = 'function:';
const NEW_PREFIX = 'new:';
const SEPARATOR = '/';

/** The option standing for what a dial already does. */
export function assignmentOption(assignment: DialAssignment): string {
	switch (assignment.kind) {
		case 'navigation':
			return NAVIGATION_OPTION;
		case 'contextual':
			return CONTEXTUAL_OPTION;
		case 'module':
			return existingOption(assignment.instance, assignment.function);
		default:
			return NO_DIAL_FUNCTION;
	}
}

export function existingOption(instance: ModuleInstanceId, fn: string): string {
	return `${EXISTING_PREFIX}${instance}${SEPARATOR}${fn}`;
}

export function newOption(module: CatalogModule, fn: DialFunctionDescriptor): string {
	return `${NEW_PREFIX}${module.kind}${SEPARATOR}${fn.id}`;
}

/** The two parts of `<first>/<second>`, `undefined` for any other text. */
function split(text: string): [string, string] | undefined {
	const at = text.indexOf(SEPARATOR);
	return at < 0 ? undefined : [text.slice(0, at), text.slice(at + 1)];
}

/** The choice an option stands for; an unknown value means none. */
export function choiceOf(value: string, modules: readonly CatalogModule[]): DialChoice {
	if (value === NAVIGATION_OPTION) return { type: 'navigation' };
	if (value === CONTEXTUAL_OPTION) return { type: 'contextual' };
	if (value.startsWith(EXISTING_PREFIX)) {
		const parts = split(value.slice(EXISTING_PREFIX.length));
		return parts ? { type: 'existing', instance: parts[0], function: parts[1] } : { type: 'none' };
	}
	if (value.startsWith(NEW_PREFIX)) {
		const parts = split(value.slice(NEW_PREFIX.length));
		const module = parts ? modules.find((candidate) => candidate.kind === parts[0]) : undefined;
		const fn = module?.dialFunctions.find((candidate) => candidate.id === parts?.[1]);
		if (module && fn) return { type: 'new', module, function: fn.id };
	}
	return { type: 'none' };
}
