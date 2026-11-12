import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'vitest-browser-svelte';
import '../../routes/layout.css';
import ModuleTile from './ModuleTile.svelte';

const PIXEL =
	'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';

async function tileBackground(theme: 'light' | 'dark') {
	document.documentElement.dataset.deviceTheme = theme;
	const { container } = await render(ModuleTile, { icon: PIXEL, name: 'Docker' });
	const tile = container.querySelector('span') as HTMLElement;
	return getComputedStyle(tile).backgroundColor;
}

describe('ModuleTile', () => {
	afterEach(() => {
		delete document.documentElement.dataset.deviceTheme;
	});

	it('is drawn on the screen of the theme of the device', async () => {
		const light = await tileBackground('light');
		const dark = await tileBackground('dark');
		expect(light).not.toBe(dark);
	});

	it('shows its name when it has no icon', async () => {
		const { container } = await render(ModuleTile, { icon: null, name: 'Docker' });
		expect(container.textContent).toContain('Docker');
	});
});
