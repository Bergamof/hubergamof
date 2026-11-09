import { describe, expect, it } from 'vitest';
import { render } from 'vitest-browser-svelte';
import BrandLogo from './BrandLogo.svelte';

describe('BrandLogo', () => {
	it('is decorative and takes the classes it is given', async () => {
		const { container } = await render(BrandLogo, { class: 'h-9 text-fg' });
		const svg = container.querySelector('svg');
		expect(svg?.getAttribute('aria-hidden')).toBe('true');
		expect(svg?.getAttribute('class')).toBe('h-9 text-fg');
	});

	it('gives each instance its own gradients, which its shapes reference', async () => {
		const first = (await render(BrandLogo, {})).container;
		const second = (await render(BrandLogo, {})).container;
		const ids = (container: Element) =>
			[...container.querySelectorAll('linearGradient')].map((gradient) => gradient.id);
		expect(ids(first)).toHaveLength(3);
		expect(ids(first).some((id) => ids(second).includes(id))).toBe(false);
		for (const id of ids(first)) {
			expect(first.querySelector(`[fill="url(#${id})"]`)).not.toBeNull();
		}
	});
});
