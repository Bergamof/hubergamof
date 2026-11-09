import { describe, expect, it } from 'vitest';
import { render } from 'vitest-browser-svelte';
import BrandWordmark from './BrandWordmark.svelte';

describe('BrandWordmark', () => {
	it('is decorative and takes the classes it is given', async () => {
		const { container } = await render(BrandWordmark, { class: 'h-9 text-fg' });
		const svg = container.querySelector('svg');
		expect(svg?.getAttribute('aria-hidden')).toBe('true');
		expect(svg?.getAttribute('class')).toBe('h-9 text-fg');
	});

	it('has a transparent background: only the three parts of the name are drawn', async () => {
		const { container } = await render(BrandWordmark, {});
		expect(container.querySelectorAll('svg > g > path')).toHaveLength(3);
		expect(container.querySelector('svg > rect')).toBeNull();
	});

	it('gives each instance its own gradient, which the B references', async () => {
		const first = (await render(BrandWordmark, {})).container;
		const second = (await render(BrandWordmark, {})).container;
		const gradientId = (container: Element) => container.querySelector('linearGradient')?.id;
		expect(gradientId(first)).not.toBe(gradientId(second));
		expect(first.querySelector(`[fill="url(#${gradientId(first)})"]`)).not.toBeNull();
	});

	it('draws the rest of the name in the current text colour', async () => {
		const { container } = await render(BrandWordmark, {});
		expect(container.querySelector('svg > g > path:last-child')?.getAttribute('fill')).toBe(
			'currentColor'
		);
	});
});
