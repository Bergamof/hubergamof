/** The parts of the page: the Stream Deck editor, the mouse editor, and the radial menu shared by the mice. */
export type Section = 'deck' | 'mouse' | 'menu';

export const SECTION_LABELS: Readonly<Record<Section, string>> = {
	deck: 'Stream Deck',
	mouse: 'Souris',
	menu: 'Menu circulaire'
};

/** What can be shown: a kind of device when one is connected, the radial menu always. */
export function availableSections(hasDecks: boolean, hasMice: boolean): Section[] {
	return [
		...(hasDecks ? (['deck'] as const) : []),
		...(hasMice ? (['mouse'] as const) : []),
		'menu'
	];
}

/** The section shown: the one asked for when it is available, else the first one. */
export function sectionShown(wanted: Section, available: readonly Section[]): Section {
	return available.includes(wanted) ? wanted : available[0];
}
