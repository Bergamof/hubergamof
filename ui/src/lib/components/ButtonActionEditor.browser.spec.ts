import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ButtonAction } from '#lib/generated/index.js';
import ButtonActionEditor from './ButtonActionEditor.svelte';
import { CATALOG } from './mouse-fixtures.js';

async function renderEditor(action?: ButtonAction) {
	const onchange = vi.fn();
	await render(ButtonActionEditor, { action, catalog: CATALOG, onchange });
	return onchange;
}

const KIND = page.getByLabelText('Action', { exact: true });

describe('ButtonActionEditor', () => {
	it('shows the behaviour of the mouse when the control has no action', async () => {
		await renderEditor();
		await expect.element(KIND).toHaveValue('native');
		expect(document.body.textContent).not.toContain('Rétablir');
	});

	it.each([
		['none', { kind: 'none' }],
		['doubleClick', { kind: 'doubleClick' }],
		['toggleWheelMode', { kind: 'toggleWheelMode' }],
		['radialMenu', { kind: 'radialMenu' }],
		['click', { kind: 'click', button: 'middle' }],
		['module', { kind: 'module', module: 'pomodoro', action: 'toggle' }]
	])('gives the action %s', async (kind, expected) => {
		const onchange = await renderEditor();
		await KIND.selectOptions(kind);
		expect(onchange).toHaveBeenCalledExactlyOnceWith(expected);
	});

	it('gives the control back to the mouse', async () => {
		const onchange = await renderEditor({ kind: 'doubleClick' });
		await KIND.selectOptions('native');
		expect(onchange).toHaveBeenCalledExactlyOnceWith(null);
	});

	it('restores the original action with the button made for it', async () => {
		const onchange = await renderEditor({ kind: 'doubleClick' });
		await page.getByRole('button', { name: 'Rétablir l’action d’origine' }).click();
		expect(onchange).toHaveBeenCalledExactlyOnceWith(null);
	});

	it('chooses the button a click presses', async () => {
		const onchange = await renderEditor({ kind: 'click', button: 'left' });
		await page.getByLabelText('Bouton cliqué').selectOptions('forward');
		expect(onchange).toHaveBeenCalledExactlyOnceWith({ kind: 'click', button: 'forward' });
	});

	it('offers only modules with actions, and starts with the first action of the module chosen', async () => {
		const onchange = await renderEditor({ kind: 'module', module: 'pomodoro', action: 'skip' });
		const module = page.getByLabelText('Module');
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
		const onchange = await renderEditor({ kind: 'module', module: 'pomodoro', action: 'skip' });
		const action = page.getByLabelText('Action du module');
		await expect.element(action).toHaveValue('skip');
		await action.selectOptions('toggle');
		expect(onchange).toHaveBeenCalledExactlyOnceWith({
			kind: 'module',
			module: 'pomodoro',
			action: 'toggle'
		});
	});

	it('keeps showing a module or an action that no longer exists', async () => {
		await renderEditor({ kind: 'module', module: 'gone', action: 'old' });
		await expect.element(page.getByLabelText('Module')).toHaveValue('gone');
		await expect.element(page.getByRole('option', { name: 'gone (absent)' })).toBeInTheDocument();
		await renderEditor({ kind: 'module', module: 'pomodoro', action: 'old' });
		await expect.element(page.getByRole('option', { name: 'old (absente)' })).toBeInTheDocument();
	});

	it('waits for the keys of a shortcut before giving the action', async () => {
		const onchange = await renderEditor();
		await KIND.selectOptions('shortcut');
		expect(onchange).not.toHaveBeenCalled();
		await expect
			.element(page.getByRole('button', { name: 'Rétablir l’action d’origine' }))
			.toBeVisible();
		await page.getByLabelText('Raccourci', { exact: true }).fill('ctrl+m');
		await page.getByLabelText('Raccourci', { exact: true }).click();
		await KIND.click();
		expect(onchange).toHaveBeenCalledExactlyOnceWith({ kind: 'shortcut', keys: 'ctrl+m' });
	});

	it('shows the shortcut of the control', async () => {
		await renderEditor({ kind: 'shortcut', keys: 'alt+left' });
		await expect.element(page.getByLabelText('Raccourci', { exact: true })).toHaveValue('alt+left');
	});
});
