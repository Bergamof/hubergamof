import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { DialSummary } from '#lib/deck/display.js';
import DialRow from './DialRow.svelte';

function dial(name: string, overrides: Partial<DialSummary> = {}): DialSummary {
	return { name, rotate: name, press: null, assigned: true, missing: false, ...overrides };
}

const FREE = dial('Rien', { rotate: null, assigned: false });

async function renderRow(dials: readonly DialSummary[], selected?: number) {
	const onselect = vi.fn();
	const { container } = await render(DialRow, { dials, selected, onselect });
	return { container, onselect };
}

describe('DialRow', () => {
	it('names each dial by its function, a free one as such', async () => {
		await renderRow([dial('Contrôle du volume'), FREE]);
		await expect
			.element(page.getByRole('button', { name: 'Molette 1 : Contrôle du volume' }))
			.toBeVisible();
		await expect.element(page.getByRole('button', { name: 'Molette 2 : Rien' })).toBeVisible();
	});

	it('draws a knob per dial, with the notch lit for an assigned one only', async () => {
		const { container } = await renderRow([dial('Navigation'), FREE]);
		expect(container.querySelectorAll('img')).toHaveLength(0);
		const knobs = [...container.querySelectorAll('svg[data-assigned]')];
		expect(knobs.map((knob) => knob.getAttribute('data-assigned'))).toEqual(['true', 'false']);
	});

	it('marks the selected dial current and reports a click', async () => {
		const { onselect } = await renderRow([dial('Navigation'), dial('Fonction contextuelle')], 1);
		const second = page.getByRole('button', { name: 'Molette 2 : Fonction contextuelle' });
		await expect.element(second).toHaveAttribute('aria-current', 'true');
		await page.getByRole('button', { name: 'Molette 1 : Navigation' }).click();
		expect(onselect).toHaveBeenCalledWith(0);
	});

	it('flags a function of a module this manager does not provide', async () => {
		await renderRow([dial('volume', { missing: true })]);
		await expect.element(page.getByText('indisponible')).toBeVisible();
	});
});
