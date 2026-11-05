import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { playwright } from '@vitest/browser-playwright';

/** Minimum coverage (%) of `pnpm test:coverage`: measured minus 2 points, never lowered. */
const COVERAGE_FLOOR = { lines: 53, functions: 59, branches: 58, statements: 55 };

/** Address of the Rust manager (default port) that `pnpm dev` proxies to. */
const MANAGER_URL = 'http://127.0.0.1:7878';

export default defineConfig({
	plugins: [
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			// The manager binary embeds `build/` and serves `200.html` (SPA shell) for unknown routes.
			// Not `index.html`: it would overwrite the prerendered home page.
			// `precompress`: `.br`/`.gz` next to each text file, served to clients accepting them.
			adapter: adapter({ fallback: '200.html', precompress: true })
		})
	],
	server: {
		proxy: {
			'/ws': { target: MANAGER_URL, ws: true },
			'/api': { target: MANAGER_URL }
		}
	},
	test: {
		expect: { requireAssertions: true },
		projects: [
			{
				extends: true,
				test: {
					name: 'unit',
					environment: 'node',
					include: ['src/**/*.{test,spec}.{js,ts}'],
					exclude: ['src/**/*.browser.spec.ts']
				}
			},
			{
				// Components in a real Chromium (the one of Playwright, headless).
				extends: true,
				test: {
					name: 'components',
					include: ['src/**/*.browser.spec.ts'],
					browser: {
						enabled: true,
						headless: true,
						provider: playwright(),
						instances: [{ browser: 'chromium' }]
					}
				}
			}
		],
		coverage: {
			provider: 'v8',
			include: ['src/**/*.{ts,svelte}'],
			exclude: ['src/lib/generated/**', 'src/**/*.spec.ts', 'src/**/*.d.ts'],
			reporter: ['text-summary', 'json-summary'],
			// Ratchet: measured coverage minus 2 points, raised as tests are added.
			thresholds: COVERAGE_FLOOR
		}
	}
});
