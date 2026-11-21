import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ButtonAction } from '#lib/generated/index.js';
import ButtonBindingRow from './ButtonBindingRow.svelte';
import { CATALOG } from './mouse-fixtures.js';

async function renderRow(action?: ButtonAction, locked = false) {
	const onchange = vi.fn();
	await render(ButtonBindingRow, {
		label: 'Bouton arrière',
		locked,
		action,
		catalog: CATALOG,
		onchange
	});
	return onchange;
}

const KIND = page.getByLabelText('Action de : Bouton arrière');

describe('ButtonBindingRow', () => {
	it('shows the behaviour of the mouse when the control has no action', async () => {
		await renderRow();
		await expect.element(KIND).toHaveValue('native');
	});

	it('only names a locked control', async () => {
		await renderRow(undefined, true);
		await expect.element(page.getByText('Bouton arrière')).toBeVisible();
		await expect.element(page.getByText(/Garde toujours son rôle/)).toBeVisible();
		expect(document.querySelector('select')).toBeNull();
	});

	it.each([
		['none', { kind: 'none' }],
		['doubleClick', { kind: 'doubleClick' }],
		['toggleWheelMode', { kind: 'toggleWheelMode' }],
		['radialMenu', { kind: 'radialMenu' }],
		['click', { kind: 'click', button: 'middle' }],
		['module', { kind: 'module', module: 'pomodoro', action: 'toggle' }]
	])('gives the action %s', async (kind, expected) => {
		const onchange = await renderRow();
		await KIND.selectOptions(kind);
		expect(onchange).toHaveBeenCalledExactlyOnceWith(expected);
	});

	it('gives the control back to the mouse', async () => {
		const onchange = await renderRow({ kind: 'doubleClick' });
		await KIND.selectOptions('native');
		expect(onchange).toHaveBeenCalledExactlyOnceWith(null);
	});

	it('chooses the button a click presses', async () => {
		const onchange = await renderRow({ kind: 'click', button: 'left' });
		await page.getByLabelText('Bouton cliqué par : Bouton arrière').selectOptions('forward');
		expect(onchange).toHaveBeenCalledExactlyOnceWith({ kind: 'click', button: 'forward' });
	});

	it('offers only modules with actions, and starts with the first action of the module chosen', async () => {
		const onchange = await renderRow({ kind: 'module', module: 'pomodoro', action: 'skip' });
		const module = page.getByLabelText('Module de : Bouton arrière');
		await expect.element(module).toHaveValue('pomodoro');
		expect(
			document.querySelectorAll('#' + CSS.escape(module.element().id) + ' option')
		).toHaveLength(2);
		await module.selectOptions('audio');
		expect(onchange).toHaveBeenCalledExactlyOnceWith({
			kind: 'module',
			module: 'audio',
			action: 'mute-microphone'
		});
	});

	it('chooses the action of the module', async () => {
		const onchange = await renderRow({ kind: 'module', module: 'pomodoro', action: 'skip' });
		const action = page.getByLabelText('Action du module de : Bouton arrière');
		await expect.element(action).toHaveValue('skip');
		await action.selectOptions('toggle');
		expect(onchange).toHaveBeenCalledExactlyOnceWith({
			kind: 'module',
			module: 'pomodoro',
			action: 'toggle'
		});
	});

	it('keeps showing a module or an action that no longer exists', async () => {
		await renderRow({ kind: 'module', module: 'gone', action: 'old' });
		await expect.element(page.getByLabelText('Module de : Bouton arrière')).toHaveValue('gone');
		await expect.element(page.getByRole('option', { name: 'gone (absent)' })).toBeInTheDocument();
		await renderRow({ kind: 'module', module: 'pomodoro', action: 'old' });
		await expect.element(page.getByRole('option', { name: 'old (absente)' })).toBeInTheDocument();
	});

	it('waits for the keys of a shortcut before giving the action', async () => {
		const onchange = await renderRow();
		await KIND.selectOptions('shortcut');
		expect(onchange).not.toHaveBeenCalled();
		await page.getByLabelText('Raccourci de : Bouton arrière').fill('ctrl+m');
		await page.getByLabelText('Raccourci de : Bouton arrière').click();
		await page.getByLabelText('Action de : Bouton arrière').click();
		expect(onchange).toHaveBeenCalledExactlyOnceWith({ kind: 'shortcut', keys: 'ctrl+m' });
	});

	it('shows the shortcut of the control', async () => {
		await renderRow({ kind: 'shortcut', keys: 'alt+left' });
		await expect
			.element(page.getByLabelText('Raccourci de : Bouton arrière'))
			.toHaveValue('alt+left');
	});
});
