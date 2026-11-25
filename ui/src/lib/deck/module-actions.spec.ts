import { describe, expect, it } from 'vitest';
import type { ApplicationModuleView, CatalogModule } from '#lib/generated/index.js';
import { availableActions, groupActions, isEnding, noActionText } from './module-actions.js';

const action = (id: string) => ({ id, name: id, description: '' });
const TEAMS = {
	kind: 'teams',
	radialActions: ['mute', 'camera', 'hang-up', 'conversations', 'teams', 'activity'].map(action)
} as unknown as CatalogModule;
const view = (availableActions?: string[] | null): ApplicationModuleView => ({
	kind: 'teams',
	settings: {},
	...(availableActions === undefined ? {} : { availableActions })
});

describe('availableActions', () => {
	it('offers every action of a module that says nothing', () => {
		expect(availableActions(TEAMS, view()).map((a) => a.id)).toHaveLength(6);
		expect(availableActions(TEAMS, view(null))).toHaveLength(6);
	});

	it('offers only those the manager says make sense, in the order of the module', () => {
		const ids = availableActions(TEAMS, view(['activity', 'mute'])).map((a) => a.id);
		expect(ids).toEqual(['mute', 'activity']);
		expect(availableActions(TEAMS, view([]))).toEqual([]);
	});

	it('offers nothing for a module the catalog does not know', () => {
		expect(availableActions(undefined, view())).toEqual([]);
	});
});

describe('groupActions', () => {
	it('puts the controls of a call apart from the panels, and leaves empty groups out', () => {
		const groups = groupActions('teams', TEAMS.radialActions);
		expect(groups.map((g) => [g.label, g.actions.map((a) => a.id)])).toEqual([
			['Appel en cours', ['mute', 'camera', 'hang-up']],
			['Ouvrir dans Teams', ['conversations', 'teams', 'activity']]
		]);
		const idle = groupActions('teams', TEAMS.radialActions.slice(3));
		expect(idle.map((g) => g.label)).toEqual(['Ouvrir dans Teams']);
	});

	it('keeps the actions of another module together, with no name', () => {
		const actions = ['start', 'pause'].map(action);
		expect(groupActions('pomodoro', actions)).toEqual([{ label: undefined, actions }]);
		expect(groupActions('pomodoro', [])).toEqual([]);
	});
});

describe('words', () => {
	it('knows what ends something', () => {
		expect(isEnding(action('hang-up'))).toBe(true);
		expect(isEnding(action('mute'))).toBe(false);
	});

	it('says why there is no action', () => {
		expect(noActionText('teams')).toMatch(/Teams/);
		expect(noActionText('audio')).toMatch(/Aucune action/);
	});
});
