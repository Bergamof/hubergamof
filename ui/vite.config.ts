import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';

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
			adapter: adapter({ fallback: '200.html' })
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
		environment: 'node',
		include: ['src/**/*.{test,spec}.{js,ts}']
	}
});
