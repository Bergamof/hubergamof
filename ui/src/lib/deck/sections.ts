/** The parts of the page: the Stream Deck editor, the mouse editor, the radial menu shared by the mice, the modules of the application and the machine. */
export type Section = 'deck' | 'mouse' | 'menu' | 'modules' | 'machine';

export const SECTION_LABELS: Readonly<Record<Section, string>> = {
	deck: 'Stream Deck',
	mouse: 'Souris',
	menu: 'Menu circulaire',
	modules: 'Modules',
	machine: 'Machine'
};

/**
 * What can be shown: a kind of device when one is connected, the radial menu and the machine
 * always, the modules of the application when the manager has some.
 */
export function availableSections(
	hasDecks: boolean,
	hasMice: boolean,
	hasModules = false
): Section[] {
	return [
		...(hasDecks ? (['deck'] as const) : []),
		...(hasMice ? (['mouse'] as const) : []),
		'menu',
		...(hasModules ? (['modules'] as const) : []),
		'machine'
	];
}

/** The section shown: the one asked for when it is available, else the first one. */
export function sectionShown(wanted: Section, available: readonly Section[]): Section {
	return available.includes(wanted) ? wanted : available[0];
}
