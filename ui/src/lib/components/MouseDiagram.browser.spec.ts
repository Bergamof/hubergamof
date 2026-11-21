import { describe, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { MouseControlId, MouseModel } from '#lib/generated/index.js';
import { MOUSE_ART } from './mouse-art.js';
import MouseDiagram from './MouseDiagram.svelte';

const SUMMARIES = { back: 'Raccourci Ctrl + Maj + M', left: 'Clic gauche' };

async function renderDiagram(
	model: MouseModel = 'mxMaster4',
	selected: MouseControlId | null = 'back'
) {
	const onselect = vi.fn();
	await render(MouseDiagram, {
		model,
		name: model === 'mxMaster4' ? 'MX Master 4' : 'MX Anywhere 3S',
		selected: selected ?? undefined,
		locked: ['left', 'right'],
		summaries: SUMMARIES,
		onselect
	});
	return onselect;
}

const marker = (id: string) => document.querySelector(`.mk[data-id="${id}"]`) as SVGGElement;
const zone = (id: string) => document.querySelector(`.part[data-id="${id}"]`) as SVGElement;

describe('MouseDiagram', () => {
	it('draws the two views of the model, each described by its name', async () => {
		await renderDiagram();
		await expect
			.element(page.getByRole('group', { name: 'MX Master 4, vue de dessus' }))
			.toBeVisible();
		await expect
			.element(page.getByRole('group', { name: 'MX Master 4, côté pouce (face gauche)' }))
			.toBeVisible();
	});

	it('puts one marker on each control of the model, and a zone on each one that has a drawing', async () => {
		await renderDiagram();
		expect(document.querySelectorAll('.mk')).toHaveLength(MOUSE_ART.mxMaster4.spots.length);
		expect(zone('hapticPanel')).not.toBeNull();
		await renderDiagram('mxAnywhere3S');
		expect(MOUSE_ART.mxAnywhere3S.spots).toHaveLength(6);
	});

	it('reads each marker with its number, its control and what it does', async () => {
		await renderDiagram();
		expect(marker('back').getAttribute('aria-label')).toBe(
			'5, Précédent : Raccourci Ctrl + Maj + M'
		);
		expect(marker('forward').getAttribute('aria-label')).toBe('6, Suivant : ');
	});

	it('marks the control chosen, and shows the locked ones apart', async () => {
		await renderDiagram();
		expect(marker('back').getAttribute('aria-current')).toBe('true');
		expect(marker('forward').hasAttribute('aria-current')).toBe(false);
		expect(marker('left').classList.contains('locked')).toBe(true);
		expect(marker('back').classList.contains('locked')).toBe(false);
		expect(zone('back').classList.contains('sel')).toBe(true);
		expect(zone('left').classList.contains('locked')).toBe(true);
	});

	it('chooses the control pressed, on its marker or on its zone', async () => {
		const onselect = await renderDiagram();
		marker('forward').dispatchEvent(new MouseEvent('click', { bubbles: true }));
		expect(onselect).toHaveBeenLastCalledWith('forward');
		zone('gesture').dispatchEvent(new MouseEvent('click', { bubbles: true }));
		expect(onselect).toHaveBeenLastCalledWith('gesture');
	});

	it('chooses a marker with Enter or Space, and ignores the other keys', async () => {
		const onselect = await renderDiagram();
		marker('wheelClick').focus();
		await userEvent.keyboard('{Enter}');
		expect(onselect).toHaveBeenLastCalledWith('wheelClick');
		onselect.mockClear();
		await userEvent.keyboard(' ');
		expect(onselect).toHaveBeenLastCalledWith('wheelClick');
		onselect.mockClear();
		await userEvent.keyboard('a');
		expect(onselect).not.toHaveBeenCalled();
	});

	it('chooses nothing in particular when nothing is chosen', async () => {
		await renderDiagram('mxMaster4', null);
		expect(document.querySelector('.mk[aria-current]')).toBeNull();
	});
});
