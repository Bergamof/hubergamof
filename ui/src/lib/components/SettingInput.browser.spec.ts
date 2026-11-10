import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
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
	const onedit = vi.fn();
	await render(SettingInput, {
		field: setting,
		idPrefix: 'form',
		value,
		error,
		serial: 'DECK',
		api: {} as ManagerApi,
		moduleKind: 'sample',
		defaultIcon: null,
		onedit,
		onediting: vi.fn()
	});
	return { onedit };
}

describe('SettingInput', () => {
	it('shows a boolean as a checkbox named by its label, and toggles it', async () => {
		await show(field({ type: 'boolean', default: false }), false);
		const box = page.getByRole('checkbox', { name: 'Réglage' });
		await expect.element(box).not.toBeChecked();
		await box.click();
		await expect.element(box).toBeChecked();
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

	it('offers the options of a choice', async () => {
		await show(
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
	});

	it('types a text, and a path of a folder is explained as one', async () => {
		await show(field({ type: 'text', default: '' }), 'Travail');
		await expect.element(page.getByLabelText('Réglage')).toHaveValue('Travail');
	});

	it('hints at the kind of path wanted', async () => {
		await show(field({ type: 'path', default: '', directory: true }), '~/projets');
		await expect.element(page.getByText(/chemin absolu d’un dossier/i)).toBeVisible();
	});

	it('takes a list of paths one per line', async () => {
		await show(field({ type: 'pathList', default: [] }), '/a.yaml\n/b.yaml');
		await expect.element(page.getByLabelText('Réglage')).toHaveValue('/a.yaml\n/b.yaml');
		await expect.element(page.getByText(/Un chemin absolu par ligne/)).toBeVisible();
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
