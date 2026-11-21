import { describe, expect, it } from 'vitest';
import { loadKeyboardLayout } from './keyboard-layout.js';

describe('loadKeyboardLayout', () => {
	it('gives the layout the browser tells', async () => {
		const layout = new Map([['KeyA', 'q']]);
		expect(await loadKeyboardLayout({ getLayoutMap: () => Promise.resolve(layout) })).toBe(layout);
	});

	it('gives nothing where the browser cannot tell, or refuses', async () => {
		expect(await loadKeyboardLayout(undefined)).toBeUndefined();
		expect(await loadKeyboardLayout({})).toBeUndefined();
		expect(
			await loadKeyboardLayout({ getLayoutMap: () => Promise.reject(new Error('denied')) })
		).toBeUndefined();
	});
});
