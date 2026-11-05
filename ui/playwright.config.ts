import { defineConfig, devices } from '@playwright/test';

/** Port of `vite preview` (its default), serving the production build with no manager behind. */
const PREVIEW_PORT = 4173;
const isCi = Boolean(process.env.CI);

export default defineConfig({
	testDir: 'e2e',
	testMatch: '**/*.e2e.ts',
	// Builds the UI and the manager; each demo test then starts its own (`e2e/demo-manager.ts`).
	globalSetup: './e2e/global-setup.ts',
	forbidOnly: isCi,
	retries: isCi ? 1 : 0,
	// A test passing only on its retry fails the run: retried, but never hidden.
	failOnFlakyTests: isCi,
	reporter: isCi ? [['list'], ['html', { open: 'never' }]] : 'list',
	projects: [
		{
			name: 'sans-gestionnaire',
			testMatch: '**/offline.e2e.ts',
			use: { ...devices['Desktop Chrome'], baseURL: `http://127.0.0.1:${PREVIEW_PORT}` }
		},
		{
			name: 'demo',
			testMatch: '**/demo.e2e.ts',
			use: { ...devices['Desktop Chrome'] }
		}
	],
	// Builds then serves the UI: Playwright starts this server before the global setup runs,
	// so a clean checkout (CI) has no build yet.
	webServer: {
		command: `pnpm build && pnpm preview --host 127.0.0.1 --port ${PREVIEW_PORT} --strictPort`,
		url: `http://127.0.0.1:${PREVIEW_PORT}`,
		reuseExistingServer: !isCi
	}
});
