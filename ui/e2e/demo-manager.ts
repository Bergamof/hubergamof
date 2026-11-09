import { spawn, type ChildProcess } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test as base } from '@playwright/test';

export const REPOSITORY_ROOT = fileURLToPath(new URL('../../', import.meta.url));
const MANAGER_BINARY = join(REPOSITORY_ROOT, 'target', 'debug', 'hubergamof');
/** Logged by the manager once it serves the interface. */
const READY_LOG = 'web interface ready';
const START_TIMEOUT_MS = 30_000;

/** A port nobody listens on right now. */
async function freePort(): Promise<number> {
	const server = createServer();
	server.listen(0, '127.0.0.1');
	await once(server, 'listening');
	const address = server.address();
	server.close();
	if (address === null || typeof address === 'string') throw new Error('no TCP port');
	return address.port;
}

async function waitUntilReady(manager: ChildProcess): Promise<void> {
	let logs = '';
	await new Promise<void>((resolve, reject) => {
		const timer = setTimeout(
			() => reject(new Error(`manager not ready after ${START_TIMEOUT_MS} ms:\n${logs}`)),
			START_TIMEOUT_MS
		);
		const read = (chunk: Buffer) => {
			logs += chunk.toString();
			if (logs.includes(READY_LOG)) {
				clearTimeout(timer);
				resolve();
			}
		};
		manager.stdout?.on('data', read);
		manager.stderr?.on('data', read);
		manager.once('exit', (code) => {
			clearTimeout(timer);
			reject(new Error(`manager exited with ${code}:\n${logs}`));
		});
	});
}

/**
 * Tests against their own `hubergamof --demo`, started on a free port with a fresh
 * configuration (the default layout of a Stream Deck+): tests are independent, and a retry
 * starts from scratch.
 */
/** File the manager answers to the interface's file dialog: no desktop dialog in tests. */
export const DEMO_COMPOSE_FILE = '/srv/demo/compose.yaml';

export const test = base.extend({
	baseURL: async ({ browserName }, use) => {
		const configDir = await mkdtemp(join(tmpdir(), `hubergamof-e2e-${browserName}-`));
		const port = await freePort();
		const manager = spawn(
			MANAGER_BINARY,
			[
				'--demo',
				'--no-tray',
				'--file-chooser-answer',
				DEMO_COMPOSE_FILE,
				'--port',
				String(port),
				'--config-dir',
				configDir
			],
			{ stdio: ['ignore', 'pipe', 'pipe'] }
		);
		try {
			await waitUntilReady(manager);
			await use(`http://127.0.0.1:${port}`);
		} finally {
			manager.kill('SIGTERM');
			if (manager.exitCode === null && manager.signalCode === null) await once(manager, 'exit');
			await rm(configDir, { recursive: true, force: true });
		}
	}
});

export { expect } from '@playwright/test';
