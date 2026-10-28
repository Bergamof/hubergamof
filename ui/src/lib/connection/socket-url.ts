/** Path of the manager WebSocket endpoint (proxied by `pnpm dev`, served by the manager otherwise). */
export const MANAGER_SOCKET_PATH = '/ws';

/** Builds the manager WebSocket URL from the page location, matching its security level. */
export function managerSocketUrl(location: Pick<Location, 'protocol' | 'host'>): string {
	const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
	return `${scheme}//${location.host}${MANAGER_SOCKET_PATH}`;
}
