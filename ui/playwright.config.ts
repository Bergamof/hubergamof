import { defineConfig, devices } from '@playwright/test';

/** Port of `vite preview` (its default), serving the production build. */
const PREVIEW_PORT = 4173;
const isCi = Boolean(process.env.CI);

export default defineConfig({
	testMatch: '**/*.e2e.{ts,js}',
	forbidOnly: isCi,
	retries: isCi ? 1 : 0,
	reporter: isCi ? [['list'], ['html', { open: 'never' }]] : 'list',
	use: { baseURL: `http://127.0.0.1:${PREVIEW_PORT}` },
	projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
	webServer: {
		command: `pnpm build && pnpm preview --host 127.0.0.1 --port ${PREVIEW_PORT} --strictPort`,
		url: `http://127.0.0.1:${PREVIEW_PORT}`,
		reuseExistingServer: !isCi
	}
});
