import type {
	ApplicationModuleView,
	CatalogModule,
	RadialActionDescriptor
} from '#lib/generated/index.js';

/** The actions of a module as the page shows them: those that make sense now, in a group. */
export interface ActionGroup {
	/** What the group is about, when the module has several. */
	label: string | undefined;
	actions: RadialActionDescriptor[];
}

/** How a module's actions are grouped, by action id; the others go in a last group with no name. */
const GROUPS: Record<string, { label: string; ids: readonly string[] }[]> = {
	teams: [
		{ label: 'Appel en cours', ids: ['mute', 'camera', 'hang-up', 'return-to-call'] },
		{ label: 'Ouvrir dans Teams', ids: ['conversations', 'teams', 'activity'] }
	]
};

/** Actions that end something: they are not drawn as the main action of the module. */
const ENDING: ReadonlySet<string> = new Set(['hang-up']);

/**
 * The actions of `module` that make sense now: those the manager says (a module that says
 * nothing offers all of them), in the order the module declares them.
 */
export function availableActions(
	module: CatalogModule | undefined,
	view: ApplicationModuleView
): RadialActionDescriptor[] {
	if (!module) return [];
	const offered = view.availableActions;
	if (offered === undefined || offered === null) return module.radialActions;
	return module.radialActions.filter((action) => offered.includes(action.id));
}

/** The available actions in groups, empty ones left out. */
export function groupActions(
	kind: string,
	actions: readonly RadialActionDescriptor[]
): ActionGroup[] {
	const named = GROUPS[kind];
	if (!named) return actions.length > 0 ? [{ label: undefined, actions: [...actions] }] : [];
	const placed = new Set(named.flatMap((group) => group.ids));
	const groups: ActionGroup[] = named.map((group) => ({
		label: group.label,
		actions: actions.filter((action) => group.ids.includes(action.id))
	}));
	groups.push({
		label: undefined,
		actions: actions.filter((action) => !placed.has(action.id))
	});
	return groups.filter((group) => group.actions.length > 0);
}

/** Whether the action ends something (hang up). */
export function isEnding(action: RadialActionDescriptor): boolean {
	return ENDING.has(action.id);
}

/** What to say when no action is offered: the module is not in a state where any makes sense. */
export function noActionText(kind: string): string {
	return kind === 'teams'
		? 'Aucune action pour l’instant : Teams n’est pas ouvert dans le navigateur, ou son extension n’est pas connectée.'
		: 'Aucune action disponible pour l’instant.';
}
