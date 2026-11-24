import { describe, expect, it } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import { ADVANCED_MAX } from '#lib/deck/advanced-validation.js';
import type { AdvancedSetting } from '#lib/generated/index.js';
import Harness from './HostOptionsEditorHarness.svelte';

function option(key: string, value = 'yes', enabled = true): AdvancedSetting {
	return { key, value, enabled };
}

async function renderEditor(initial: AdvancedSetting[] = [], host = '') {
	const state = { options: initial, committed: [] as boolean[] };
	await render(Harness, {
		initial,
		host,
		onoptions: (next: AdvancedSetting[]) => (state.options = next),
		oncommit: (answer: boolean) => state.committed.push(answer)
	});
	return state;
}

const keyField = () => page.getByRole('textbox', { name: 'Nom de l’option' });
const valueField = () => page.getByRole('textbox', { name: 'Valeur' });
const sent = () => page.getByLabelText('Envois');

async function fill(key: string, value: string) {
	await keyField().fill(key);
	await valueField().fill(value);
}

describe('HostOptionsEditor', () => {
	it('adds an option with its button and starts again from the name', async () => {
		const state = await renderEditor();
		await fill('Compression', 'yes');
		await page.getByRole('button', { name: 'Ajouter l’option' }).click();
		expect(state.options).toEqual([option('Compression')]);
		await expect.element(keyField()).toHaveFocus();
		await expect.element(page.getByRole('list', { name: 'Options' })).toBeVisible();
	});

	it('adds the option on Enter and does not send the form that holds it', async () => {
		const state = await renderEditor();
		await fill('LogLevel', 'ERROR');
		await valueField().click();
		await userEvent.keyboard('{Enter}');
		expect(state.options).toEqual([option('LogLevel', 'ERROR')]);
		await expect.element(sent()).toHaveTextContent('0');
	});

	it('lets Enter send the form when the pair is empty', async () => {
		await renderEditor();
		await valueField().click();
		await userEvent.keyboard('{Enter}');
		await expect.element(sent()).toHaveTextContent('1');
		await expect.element(page.getByText(/Saisissez le nom/)).not.toBeInTheDocument();
	});

	it('says why a key is refused, with the focus on the field, and adds nothing', async () => {
		const state = await renderEditor([option('Compression')]);
		await fill('compression', 'no');
		await page.getByRole('button', { name: 'Ajouter l’option' }).click();
		await expect.element(page.getByText(/déjà dans la liste/)).toBeVisible();
		await expect.element(keyField()).toHaveFocus();
		expect(state.options).toEqual([option('Compression')]);
	});

	it('says why a value is refused, with the focus on the field', async () => {
		const state = await renderEditor();
		await fill('SendEnv', '"open');
		await page.getByRole('button', { name: 'Ajouter l’option' }).click();
		await expect.element(page.getByText(/guillemets/)).toBeVisible();
		await expect.element(valueField()).toHaveFocus();
		expect(state.options).toEqual([]);
	});

	it('names the options after the host, shows one that is off and switches it back', async () => {
		const state = await renderEditor([option('Compression'), option('LogLevel', 'ERROR')], 'prod');
		await expect.element(page.getByRole('list', { name: 'Options de prod' })).toBeVisible();
		const compression = page.getByRole('switch', { name: 'Option Compression de prod' });
		await compression.click();
		expect(state.options[0]).toEqual(option('Compression', 'yes', false));
		await expect.element(compression).not.toBeChecked();
		await compression.click();
		expect(state.options[0]).toEqual(option('Compression'));
	});

	it('gives the focus to the next option when one is removed, and says so', async () => {
		const state = await renderEditor([option('A'), option('B'), option('C')], 'prod');
		await page.getByRole('button', { name: 'Retirer l’option A de prod' }).click();
		expect(state.options.map(({ key }) => key)).toEqual(['B', 'C']);
		await expect
			.element(page.getByRole('button', { name: 'Retirer l’option B de prod' }))
			.toHaveFocus();
		await expect.element(page.getByText('Option A retirée de prod.')).toBeInTheDocument();
		await page.getByRole('button', { name: 'Retirer l’option C de prod' }).click();
		await expect
			.element(page.getByRole('button', { name: 'Retirer l’option B de prod' }))
			.toHaveFocus();
	});

	it('gives the focus back to the name field when the last option is removed', async () => {
		await renderEditor([option('A')]);
		await page.getByRole('button', { name: 'Retirer l’option A' }).click();
		await expect.element(keyField()).toHaveFocus();
	});

	it('commits a pair typed but not added when the host is sent, or says what is wrong', async () => {
		const state = await renderEditor();
		await page.getByRole('button', { name: 'Valider' }).click();
		expect(state.committed).toEqual([true]);

		await fill('Pending', '');
		await page.getByRole('button', { name: 'Valider' }).click();
		// A name without a value is refused: the host is not sent.
		expect(state.committed).toEqual([true, false]);

		await fill('Compression', 'yes');
		await page.getByRole('button', { name: 'Valider' }).click();
		expect(state.committed).toEqual([true, false, true]);
		expect(state.options).toEqual([option('Compression')]);
		await expect.element(keyField()).toHaveValue('');
	});

	it('offers no field when the list is full', async () => {
		await renderEditor(Array.from({ length: ADVANCED_MAX }, (_, index) => option(`K${index}`)));
		await expect.element(page.getByText(/nombre maximal d’éléments/)).toBeVisible();
		await expect.element(valueField()).not.toBeInTheDocument();
	});
});
