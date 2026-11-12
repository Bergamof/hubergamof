import { describe, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
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
	defaultSettings: { workMinutes: 25, sound: false },
	settings: [
		{
			key: 'workMinutes',
			label: 'Durée du travail',
			help: null,
			kind: { type: 'integer', default: 25, min: 1, max: 60, unit: 'min' }
		},
		{ key: 'sound', label: 'Son', help: null, kind: { type: 'boolean', default: false } }
	]
};

const INSTANCE: ModuleInstance = {
	id: 'pomodoro',
	kind: 'pomodoro',
	label: null,
	image: null,
	settings: { workMinutes: 25, sound: false }
};

/** Longer than the pause after which a typed text is saved. */
const AFTER_TYPING_PAUSE_MS = 2500;

type Apply = (changes: InstanceChanges) => Promise<EditOutcome>;

/**
 * Shows the editor of {@link INSTANCE}. Like the real configuration, a saved change comes back
 * as the displayed instance, unless `apply` answers otherwise.
 */
async function show(options: { apply?: Apply; module?: CatalogModule | null } = {}) {
	let current = INSTANCE;
	const onapply = vi.fn<Apply>(async (changes) => {
		const outcome: EditOutcome = options.apply ? await options.apply(changes) : { saved: true };
		if (outcome.saved) {
			current = { ...current, label: changes.label, settings: changes.settings };
			await screen.rerender({ instance: current });
		}
		return outcome;
	});
	const screen = await render(InstanceEditor, {
		instance: current,
		module: options.module === null ? undefined : POMODORO,
		serial: 'DECK',
		api: {} as ManagerApi,
		onapply
	});
	return { onapply, screen };
}

const labelField = () => page.getByLabelText('Libellé');
const workField = () => page.getByLabelText('Durée du travail');
const savedStatus = () => page.getByText('Réglages enregistrés.');

describe('InstanceEditor', () => {
	it('shows the label and the settings of the instance, with no button to apply them', async () => {
		const { onapply } = await show();
		await expect.element(labelField()).toHaveValue('');
		await expect.element(labelField()).toHaveAttribute('placeholder', 'Pomodoro');
		await expect.element(workField()).toHaveValue('25');
		await expect.element(page.getByText(/enregistrées automatiquement/)).toBeVisible();
		await expect.element(page.getByRole('button')).not.toBeInTheDocument();
		expect(onapply).not.toHaveBeenCalled();
	});

	it('saves a choice at once', async () => {
		const { onapply } = await show();
		await page.getByRole('checkbox', { name: 'Son' }).click();
		expect(onapply).toHaveBeenCalledExactlyOnceWith({
			label: null,
			settings: { workMinutes: 25, sound: true }
		});
		await expect.element(savedStatus()).toBeVisible();
	});

	it('waits for a pause in the typing before saving a text', async () => {
		const { onapply } = await show();
		await userEvent.type(labelField(), 'Focus');
		expect(onapply).not.toHaveBeenCalled();
		await expect
			.poll(() => onapply.mock.calls, { timeout: AFTER_TYPING_PAUSE_MS })
			.toEqual([[{ label: 'Focus', settings: INSTANCE.settings }]]);
		await expect.element(savedStatus()).toBeVisible();
	});

	it('saves a text at once on Enter, and only once', async () => {
		const { onapply } = await show();
		await userEvent.type(workField(), '{Backspace}{Backspace}30{Enter}');
		expect(onapply).toHaveBeenCalledExactlyOnceWith({
			label: null,
			settings: { workMinutes: 30, sound: false }
		});
		await userEvent.tab();
		await new Promise((resolve) => setTimeout(resolve, AFTER_TYPING_PAUSE_MS));
		expect(onapply).toHaveBeenCalledOnce();
	});

	it('saves a text at once when the field is left', async () => {
		const { onapply } = await show();
		await userEvent.type(labelField(), 'Focus');
		await userEvent.tab();
		expect(onapply).toHaveBeenCalledExactlyOnceWith({
			label: 'Focus',
			settings: INSTANCE.settings
		});
	});

	it('saves a text still being typed when the editor goes away', async () => {
		const { onapply, screen } = await show();
		await userEvent.type(labelField(), 'Focus');
		await screen.unmount();
		expect(onapply).toHaveBeenCalledExactlyOnceWith({
			label: 'Focus',
			settings: INSTANCE.settings
		});
	});

	it('gives back the saved value on Escape, saving nothing', async () => {
		const { onapply } = await show();
		await userEvent.type(labelField(), 'Focus{Escape}');
		await expect.element(labelField()).toHaveValue('');
		await userEvent.tab();
		expect(onapply).not.toHaveBeenCalled();
	});

	it('keeps a refused setting as typed, with its error, and saves the valid ones', async () => {
		const { onapply } = await show();
		await userEvent.type(workField(), '{Backspace}{Backspace}0');
		await userEvent.tab();
		expect(onapply).not.toHaveBeenCalled();
		await expect.element(workField()).toHaveAttribute('aria-invalid', 'true');
		await expect.element(page.getByText('Valeur minimale : 1.')).toBeVisible();
		await expect.element(page.getByText(/Réglage invalide/)).toBeVisible();

		await page.getByRole('checkbox', { name: 'Son' }).click();
		expect(onapply).toHaveBeenCalledExactlyOnceWith({
			label: null,
			settings: { workMinutes: 25, sound: true }
		});
		await expect.element(workField()).toHaveValue('0');

		await userEvent.type(workField(), '{Backspace}45{Escape}');
		await expect.element(workField()).toHaveValue('25');
		await expect.element(workField()).not.toHaveAttribute('aria-invalid');
		await expect.element(savedStatus()).toBeVisible();
	});

	it('keeps what was typed and says why when the manager refuses it', async () => {
		const { onapply } = await show({
			apply: () =>
				Promise.resolve({
					saved: false,
					problem: { text: 'Le gestionnaire a refusé la modification.', detail: 'refused' }
				})
		});
		await userEvent.type(labelField(), 'Focus{Enter}');
		await expect.element(page.getByText(/Réglages non enregistrés\./)).toBeVisible();
		await expect
			.element(page.getByText(/Le gestionnaire a refusé la modification\./))
			.toBeVisible();
		await expect.element(labelField()).toHaveValue('Focus');
		await expect.element(savedStatus()).not.toBeInTheDocument();
		await userEvent.keyboard('{Enter}');
		expect(onapply).toHaveBeenCalledTimes(2);
	});

	it('saves the changes finished while saving once, right after, with the latest values', async () => {
		const answers: ((outcome: EditOutcome) => void)[] = [];
		const { onapply } = await show({
			apply: () => new Promise((resolve) => answers.push(resolve))
		});
		await userEvent.type(labelField(), 'A{Enter}');
		await expect.element(page.getByText('Enregistrement…')).toBeVisible();
		await userEvent.type(labelField(), 'B{Enter}C{Enter}');
		expect(onapply).toHaveBeenCalledOnce();
		answers[0]?.({ saved: true });
		await expect.poll(() => onapply.mock.calls.length).toBe(2);
		expect(onapply).toHaveBeenLastCalledWith({ label: 'ABC', settings: INSTANCE.settings });
		answers[1]?.({ saved: true });
		await expect.element(savedStatus()).toBeVisible();
		expect(onapply).toHaveBeenCalledTimes(2);
	});

	it('shows a change made elsewhere in the fields left untouched', async () => {
		const { onapply, screen } = await show();
		await screen.rerender({
			instance: { ...INSTANCE, label: 'Ailleurs', settings: { workMinutes: 40, sound: false } }
		});
		await expect.element(labelField()).toHaveValue('Ailleurs');
		await expect.element(workField()).toHaveValue('40');
		expect(onapply).not.toHaveBeenCalled();
	});

	it('keeps a field being typed over a change made elsewhere, then saves it over it', async () => {
		const { onapply, screen } = await show();
		await userEvent.type(labelField(), 'Focus');
		await screen.rerender({
			instance: { ...INSTANCE, label: 'Ailleurs', settings: { workMinutes: 40, sound: false } }
		});
		await expect.element(workField()).toHaveValue('40');
		await expect.element(labelField()).toHaveValue('Focus');
		await userEvent.keyboard('{Enter}');
		expect(onapply).toHaveBeenCalledExactlyOnceWith({
			label: 'Focus',
			settings: { workMinutes: 40, sound: false }
		});
	});

	it('only offers the label for a module this version does not provide', async () => {
		await show({ module: null });
		await expect.element(page.getByText(/pas encore disponible dans cette version/)).toBeVisible();
		await expect.element(workField()).not.toBeInTheDocument();
	});
});
