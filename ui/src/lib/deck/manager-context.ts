import { createContext } from 'svelte';
import type { ManagerApi } from '#lib/api/manager-api.js';
import type { ManagerConnection } from '#lib/connection/index.js';
import type { DeckState } from '#lib/deck/deck-state.svelte.js';

/** What every page shares: the state known about the manager, its connection and its HTTP routes. */
export interface ManagerContext {
	readonly deck: DeckState;
	readonly connection: ManagerConnection;
	readonly api: ManagerApi;
}

export const [getManager, setManager] = createContext<ManagerContext>();
