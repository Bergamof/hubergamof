import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ManagerApi } from '#lib/api/manager-api.js';
import type { InstanceChanges } from '#lib/deck/config-edits.js';
import type { EditOutcome } from '#lib/deck/deck-state.svelte.js';
import type { CatalogModule, ModuleInstance } from '#lib/generated/index.js';
import InstanceEditor from './InstanceEditor.svelte';

const POMODORO: CatalogModule = {
	kind: 'pomodoro',
	name: 'Pomodoro',
	description: '',
	capabilities: { key: true, dial: true, touchZone: true },
	icon: null,
	settingIcons: {},
	defaultSettings: { workMinutes: 25 },
	settings: [
		{
			key: 'workMinutes',
			label: 'Durée du travail',
			help: null,
			kind: { type: 'integer', default: 25, min: 1, max: 60, unit: 'min' }
		}
	]
};

const INSTANCE: ModuleInstance = {
	id: 'pomodoro',
	kind: 'pomodoro',
	label: null,
	image: null,
	settings: { workMinutes: 25 }
};

async function show(
	outcome: EditOutcome = { saved: true },
	module: CatalogModule | null = POMODORO
) {
	const onapply = vi.fn<(changes: InstanceChanges) => Promise<EditOutcome>>(() =>
		Promise.resolve(outcome)
	);
	const screen = await render(InstanceEditor, {
		instance: INSTANCE,
		module: module ?? undefined,
		serial: 'DECK',
		api: {} as ManagerApi,
		onapply
	});
	return { onapply, screen };
}

const apply = () => page.getByRole('button', { name: 'Appliquer' });

describe('InstanceEditor', () => {
	it('shows the label and the settings of the instance, nothing to apply yet', async () => {
		await show();
		await expect.element(page.getByLabelText('Libellé')).toHaveValue('');
		await expect.element(page.getByLabelText('Libellé')).toHaveAttribute('placeholder', 'Pomodoro');
		await expect.element(page.getByLabelText('Durée du travail')).toHaveValue('25');
		await expect.element(apply()).toHaveAttribute('aria-disabled', 'true');
	});

	it('applies the label and the typed settings, then says they were saved', async () => {
		const { onapply } = await show();
		await page.getByLabelText('Libellé').fill('Focus');
		await page.getByLabelText('Durée du travail').fill('30');
		await expect.element(apply()).toHaveAttribute('aria-disabled', 'false');
		await apply().click();
		expect(onapply).toHaveBeenCalledExactlyOnceWith({
			label: 'Focus',
			settings: { workMinutes: 30 }
		});
		await expect.element(page.getByText('Réglages enregistrés.')).toBeVisible();
	});

	it('refuses an invalid setting without asking the manager, and focuses it', async () => {
		const { onapply } = await show();
		const work = page.getByLabelText('Durée du travail');
		await work.fill('0');
		await apply().click();
		expect(onapply).not.toHaveBeenCalled();
		await expect.element(page.getByText('Certains réglages sont invalides.')).toBeVisible();
		await expect.element(work).toHaveAttribute('aria-invalid', 'true');
		await expect.element(work).toHaveFocus();
	});

	it('keeps what was typed and says why when the manager refuses it', async () => {
		await show({
			saved: false,
			problem: { text: 'Le gestionnaire a refusé la modification.', detail: 'refused' }
		});
		await page.getByLabelText('Libellé').fill('Focus');
		await apply().click();
		await expect.element(page.getByText(/Réglages non enregistrés\./)).toBeVisible();
		await expect
			.element(page.getByText(/Le gestionnaire a refusé la modification\./))
			.toBeVisible();
		await expect.element(page.getByLabelText('Libellé')).toHaveValue('Focus');
		await expect.element(page.getByText('Réglages enregistrés.')).not.toBeInTheDocument();
	});

	it('cancels the modifications', async () => {
		await show();
		await page.getByLabelText('Libellé').fill('Focus');
		await page.getByRole('button', { name: 'Annuler les modifications' }).click();
		await expect.element(page.getByLabelText('Libellé')).toHaveValue('');
	});

	it('only offers the label for a module this version does not provide', async () => {
		await show({ saved: true }, null);
		await expect.element(page.getByText(/pas encore disponible dans cette version/)).toBeVisible();
		await expect.element(page.getByLabelText('Durée du travail')).not.toBeInTheDocument();
	});
});
