import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { createRawSnippet } from 'svelte';
import Switch from './Switch.svelte';

const text = (content: string) =>
	createRawSnippet(() => ({ render: () => `<span>${content}</span>` }));

describe('Switch', () => {
	it('is a switch named by its label, in the state it is given', async () => {
		await render(Switch, {
			checked: true,
			onchange: vi.fn(),
			children: text('Défilement naturel')
		});
		const control = page.getByRole('switch', { name: 'Défilement naturel' });
		await expect.element(control).toBeChecked();
	});

	it('reports the new state when pressed', async () => {
		const onchange = vi.fn();
		await render(Switch, { checked: false, onchange, children: text('Animer') });
		await page.getByRole('switch', { name: 'Animer' }).click();
		expect(onchange).toHaveBeenLastCalledWith(true);
	});

	it('can be named without a visible label', async () => {
		await render(Switch, { checked: false, label: 'Tout le module', onchange: vi.fn() });
		await expect.element(page.getByRole('switch', { name: 'Tout le module' })).toBeVisible();
	});

	it('does not change when disabled', async () => {
		const onchange = vi.fn();
		await render(Switch, { checked: false, disabled: true, onchange, children: text('Fixe') });
		await expect.element(page.getByRole('switch', { name: 'Fixe' })).toBeDisabled();
		expect(onchange).not.toHaveBeenCalled();
	});

	it('shows some on and some off as indeterminate, and is explained by what it points to', async () => {
		await render(Switch, {
			checked: false,
			indeterminate: true,
			describedby: 'why',
			onchange: vi.fn(),
			children: text('Module')
		});
		const control = page.getByRole('switch', { name: 'Module' });
		await expect.element(control).toHaveAttribute('aria-describedby', 'why');
		expect((control.element() as HTMLInputElement).indeterminate).toBe(true);
	});
});
