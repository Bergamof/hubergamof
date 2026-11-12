import { describe, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ManagerApi } from '#lib/api/manager-api.js';
import type { SettingField, SettingKind } from '#lib/generated/index.js';
import type { DraftValue } from '#lib/deck/settings-form.js';
import SettingInput from './SettingInput.svelte';

function field(kind: SettingKind, help: string | null = null): SettingField {
	return { key: 'setting', label: 'Réglage', help, kind };
}

async function show(
	setting: SettingField,
	value: DraftValue,
	error: string | undefined = undefined
) {
	const callbacks = { ontype: vi.fn(), oncommit: vi.fn(), onrevert: vi.fn() };
	await render(SettingInput, {
		field: setting,
		idPrefix: 'form',
		value,
		error,
		serial: 'DECK',
		api: {} as ManagerApi,
		moduleKind: 'sample',
		defaultIcon: null,
		...callbacks
	});
	return callbacks;
}

describe('SettingInput', () => {
	it('shows a boolean as a checkbox named by its label, and commits a toggle at once', async () => {
		const { oncommit, ontype } = await show(field({ type: 'boolean', default: false }), false);
		const box = page.getByRole('checkbox', { name: 'Réglage' });
		await expect.element(box).not.toBeChecked();
		await box.click();
		await expect.element(box).toBeChecked();
		expect(oncommit).toHaveBeenCalledOnce();
		expect(ontype).not.toHaveBeenCalled();
	});

	it('shows an integer with its unit and its bounds, and explains an error', async () => {
		await show(
			field({ type: 'integer', default: 25, min: 1, max: 60, unit: 'min' }, 'Durée d’un cycle.'),
			'25',
			'Valeur minimale : 1.'
		);
		const input = page.getByLabelText('Réglage');
		await expect.element(input).toHaveValue('25');
		await expect.element(input).toHaveAttribute('aria-invalid', 'true');
		await expect.element(page.getByText('min', { exact: true })).toBeVisible();
		await expect.element(page.getByText(/Durée d’un cycle\. Entre 1 et 60\./)).toBeVisible();
		await expect.element(page.getByText('Valeur minimale : 1.')).toBeVisible();
		await expect.element(input).toHaveAccessibleDescription(/Entre 1 et 60.*Valeur minimale/);
	});

	it('offers the options of a choice, and commits the one chosen at once', async () => {
		const callbacks = await show(
			field({
				type: 'choice',
				default: 'sink',
				options: [
					{ value: 'sink', label: 'Sortie' },
					{ value: 'source', label: 'Entrée' }
				]
			}),
			'sink'
		);
		const choice = page.getByRole('combobox', { name: 'Réglage' });
		await expect.element(choice).toHaveValue('sink');
		await choice.selectOptions('Entrée');
		await expect.element(choice).toHaveValue('source');
		expect(callbacks.oncommit).toHaveBeenCalledOnce();
	});

	it('reports each keystroke of a text, but commits it only on Enter or when left', async () => {
		const { ontype, oncommit } = await show(field({ type: 'text', default: '' }), 'Travail');
		const input = page.getByLabelText('Réglage');
		await expect.element(input).toHaveValue('Travail');
		await userEvent.type(input, ' du soir');
		expect(ontype).toHaveBeenCalledTimes(' du soir'.length);
		expect(oncommit).not.toHaveBeenCalled();
		await userEvent.keyboard('{Enter}');
		expect(oncommit).toHaveBeenCalledOnce();
		await userEvent.type(input, '!');
		await userEvent.tab();
		expect(oncommit).toHaveBeenCalledTimes(2);
	});

	it('asks for the saved value back on Escape', async () => {
		const { onrevert, oncommit } = await show(field({ type: 'text', default: '' }), 'Travail');
		await userEvent.type(page.getByLabelText('Réglage'), 'x{Escape}');
		expect(onrevert).toHaveBeenCalledOnce();
		expect(oncommit).not.toHaveBeenCalled();
	});

	it('hints at the kind of path wanted', async () => {
		await show(field({ type: 'path', default: '', directory: true }), '~/projets');
		await expect.element(page.getByText(/chemin absolu d’un dossier/i)).toBeVisible();
	});

	it('takes a list of paths one per line, Enter starting a new one', async () => {
		const { ontype, oncommit, onrevert } = await show(
			field({ type: 'pathList', default: [] }),
			'/a.yaml\n/b.yaml'
		);
		const input = page.getByLabelText('Réglage');
		await expect.element(input).toHaveValue('/a.yaml\n/b.yaml');
		await expect.element(page.getByText(/Un chemin absolu par ligne/)).toBeVisible();
		await userEvent.type(input, '{Enter}/c.yaml');
		await expect.element(input).toHaveValue('/a.yaml\n/b.yaml\n/c.yaml');
		expect(ontype).toHaveBeenCalled();
		expect(oncommit).not.toHaveBeenCalled();
		await userEvent.keyboard('{Escape}');
		expect(onrevert).toHaveBeenCalledOnce();
		await userEvent.tab();
		expect(oncommit).toHaveBeenCalledOnce();
	});

	it('gives a list of files its own control, named by the label', async () => {
		await show(
			field({
				type: 'fileEntryList',
				default: [],
				maxEntries: 5,
				maxNameChars: 40,
				chooser: null,
				pathPlaceholder: '/chemin',
				nameHint: 'Vide : nom du fichier.',
				defaultName: 'fileName'
			}),
			[]
		);
		await expect.element(page.getByRole('group', { name: 'Réglage' })).toBeVisible();
		await expect.element(page.getByText('Aucun fichier ajouté.')).toBeVisible();
		await expect.element(page.getByText('5 fichiers au plus.')).toBeVisible();
	});
});
