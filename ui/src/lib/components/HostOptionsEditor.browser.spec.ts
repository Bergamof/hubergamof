import { describe, expect, it } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { ADVANCED_MAX } from '#lib/deck/advanced-validation.js';
import type { AdvancedSetting } from '#lib/generated/index.js';
import HostOptionsEditor from './HostOptionsEditor.svelte';

function option(key: string, value = 'yes', enabled = true): AdvancedSetting {
	return { key, value, enabled };
}

/** The editor over a list it keeps in a variable, as the form of a host does with its draft. */
async function renderEditor(initial: AdvancedSetting[] = [], host = '') {
	const state = { options: initial };
	await render(HostOptionsEditor, {
		options: initial,
		host,
		labelledby: 'title',
		onchange: (next: AdvancedSetting[]) => (state.options = next)
	});
	return state;
}

const group = () => page.getByRole('group');

describe('HostOptionsEditor', () => {
	it('adds an option with its button and starts again from the name', async () => {
		const state = await renderEditor();
		await group().getByRole('textbox', { name: 'Nom de l’option' }).fill('Compression');
		await group().getByRole('textbox', { name: 'Valeur' }).fill('yes');
		await group().getByRole('button', { name: 'Ajouter l’option' }).click();
		expect(state.options).toEqual([option('Compression')]);
		await expect.element(group().getByRole('textbox', { name: 'Nom de l’option' })).toHaveFocus();
	});

	it('adds the option on Enter and does not send what contains it', async () => {
		let sent = false;
		const state = { options: [] as AdvancedSetting[] };
		await render(HostOptionsEditor, {
			options: [],
			labelledby: 'title',
			onchange: (next: AdvancedSetting[]) => (state.options = next)
		});
		document.addEventListener('submit', () => (sent = true));
		await group().getByRole('textbox', { name: 'Nom de l’option' }).fill('LogLevel');
		await group().getByRole('textbox', { name: 'Valeur' }).fill('ERROR');
		await page.getByRole('textbox', { name: 'Valeur' }).click();
		await userEvent.keyboard('{Enter}');
		expect(state.options).toEqual([option('LogLevel', 'ERROR')]);
		expect(sent).toBe(false);
	});

	it('says why an option is refused, with the focus on the field, and adds nothing', async () => {
		const state = await renderEditor([option('Compression')]);
		await group().getByRole('textbox', { name: 'Nom de l’option' }).fill('compression');
		await group().getByRole('textbox', { name: 'Valeur' }).fill('no');
		await group().getByRole('button', { name: 'Ajouter l’option' }).click();
		await expect.element(page.getByText(/déjà dans la liste/)).toBeVisible();
		await expect.element(group().getByRole('textbox', { name: 'Nom de l’option' })).toHaveFocus();
		expect(state.options).toEqual([option('Compression')]);
	});

	it('names the options after the host and switches or removes one', async () => {
		const state = await renderEditor([option('Compression'), option('LogLevel', 'ERROR')], 'prod');
		await expect.element(page.getByRole('list', { name: 'Options de prod' })).toBeVisible();
		await page.getByRole('switch', { name: 'Option Compression de prod' }).click();
		expect(state.options[0]).toEqual(option('Compression', 'yes', false));
		await page.getByRole('button', { name: 'Retirer l’option LogLevel de prod' }).click();
		expect(state.options.map(({ key }) => key)).toEqual(['Compression']);
	});

	it('offers no field when the list is full', async () => {
		await renderEditor(Array.from({ length: ADVANCED_MAX }, (_, index) => option(`K${index}`)));
		await expect.element(page.getByText(/nombre maximal d’éléments/)).toBeVisible();
		await expect.element(page.getByRole('textbox', { name: 'Valeur' })).not.toBeInTheDocument();
	});
});
