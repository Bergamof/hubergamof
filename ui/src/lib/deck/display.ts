import type { CatalogModule, DialAssignment, ModuleInstance } from '#lib/generated/index.js';
import { keyImageUrl } from '#lib/api/manager-api.js';

/** Name of a module kind: from the catalog, else the kind (module missing from this version). */
export function moduleName(kind: string, module: CatalogModule | undefined): string {
	return module?.name ?? kind;
}

/** Name shown for an instance: its label, else the name of its module. */
export function instanceName(instance: ModuleInstance, module: CatalogModule | undefined): string {
	return instance.label ?? moduleName(instance.kind, module);
}

/** Image shown on the key of `instance` of device `serial`: its own, else its module icon. */
export function keyImage(
	serial: string,
	instance: ModuleInstance,
	module: CatalogModule | undefined
): string | null {
	return instance.image ? keyImageUrl(serial, instance.image) : (module?.icon ?? null);
}

/** What a dial is shown as: its name, what its reminder says, and whether anything is assigned. */
export interface DialSummary {
	/** Name of the function the dial is given. */
	name: string;
	/** What turning and pressing the dial do, as the strip reminds it (a guess for a module). */
	rotate: string | null;
	press: string | null;
	/** A function is assigned. */
	assigned: boolean;
	/** The module of the function is not provided by this version of the manager. */
	missing: boolean;
}

/** Name of the dial function `id` of `module`, else the identifier. */
export function dialFunctionName(module: CatalogModule | undefined, id: string): string {
	return module?.dialFunctions.find((fn) => fn.id === id)?.name ?? id;
}

/** Summary of what `assignment` does, with the instances and modules of the configuration. */
export function dialSummary(
	assignment: DialAssignment,
	instances: ReadonlyMap<string, ModuleInstance>,
	moduleOf: (kind: string) => CatalogModule | undefined
): DialSummary {
	switch (assignment.kind) {
		case 'navigation':
			return {
				name: 'Navigation',
				rotate: 'Navigation',
				press: 'Retour',
				assigned: true,
				missing: false
			};
		case 'contextual':
			return {
				name: 'Fonction contextuelle',
				rotate: 'Selon la page',
				press: null,
				assigned: true,
				missing: false
			};
		case 'module': {
			const instance = instances.get(assignment.instance);
			const module = instance ? moduleOf(instance.kind) : undefined;
			const name = dialFunctionName(module, assignment.function);
			return { name, rotate: name, press: null, assigned: true, missing: module === undefined };
		}
		default:
			return { name: 'Rien', rotate: null, press: null, assigned: false, missing: false };
	}
}
