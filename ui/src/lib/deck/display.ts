import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
import { keyImageUrl } from '#lib/api/manager-api.js';

/** Names of the modules announced but not provided yet, shown instead of their kind. */
const UPCOMING_MODULE_NAMES: Readonly<Record<string, string>> = {
	intellij: 'IntelliJ',
	claude: 'Claude'
};

/** Name of a module kind: from the catalog, else a known upcoming name, else the kind. */
export function moduleName(kind: string, module: CatalogModule | undefined): string {
	return (
		module?.name ??
		(Object.hasOwn(UPCOMING_MODULE_NAMES, kind) ? UPCOMING_MODULE_NAMES[kind] : kind)
	);
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
