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

const DOCKER: CatalogModule = {
	...POMODORO,
	kind: 'docker',
	name: 'Docker',
	defaultSettings: { files: [] },
	settings: [
		{
			key: 'files',
			label: 'Fichiers',
			help: null,
			kind: {
				type: 'fileEntryList',
				default: [],
				maxEntries: 5,
				maxNameChars: 40,
				chooser: null,
				pathPlaceholder: '/chemin',
				nameHint: 'Vide : nom du dossier.',
				defaultName: 'folderName'
			}
		}
	]
};

const DOCKER_INSTANCE: ModuleInstance = {
	id: 'docker',
	kind: 'docker',
	label: null,
	image: null,
	settings: { files: [{ path: '/srv/shop/compose.yaml', name: 'Boutique', icon: null }] }
};

/** Longer than the pause after which a typed text is saved. */
const AFTER_TYPING_PAUSE_MS = 2500;

type Apply = (changes: InstanceChanges) => Promise<EditOutcome>;

/**
 * Shows the editor of {@link INSTANCE}. Like the real configuration, a saved change comes back
 * as the displayed instance, unless `apply` answers otherwise.
 */
async function show(
	options: { apply?: Apply; module?: CatalogModule | null; instance?: ModuleInstance } = {}
) {
	let current = options.instance ?? INSTANCE;
	let mounted = true;
	const onapply = vi.fn<Apply>(async (changes) => {
		const outcome: EditOutcome = options.apply ? await options.apply(changes) : { saved: true };
		if (outcome.saved && mounted) {
			current = { ...current, label: changes.label, settings: changes.settings };
			await screen.rerender({ instance: current });
		}
		return outcome;
	});
	const screen = await render(InstanceEditor, {
		instance: current,
		module: options.module === null ? undefined : (options.module ?? POMODORO),
		serial: 'DECK',
		api: {} as ManagerApi,
		onapply
	});
	const unmount = async () => {
		mounted = false;
		await screen.unmount();
	};
	return { onapply, screen, unmount };
}

const labelField = () => page.getByLabelText('Libellé');
const workField = () => page.getByLabelText('Durée du travail');
const savedStatus = () => page.getByText('Réglages enregistrés.');
const revertedStatus = () => page.getByText('Valeur enregistrée rétablie.');
const invalidStatus = () => page.getByText(/^Non enregistré tant que ce n’est pas corrigé/);
const REFUSAL: EditOutcome = {
	saved: false,
	problem: { text: 'Le gestionnaire a refusé la modification.', detail: 'refused' }
};
const refusalAlert = () => page.getByText(/Réglages non enregistrés\./);

describe('InstanceEditor', () => {
	it('shows the label and the settings of the instance, with no button to apply them', async () => {
		const { onapply } = await show();
		await expect.element(labelField()).toHaveValue('');
		await expect.element(labelField()).toHaveAttribute('placeholder', 'Pomodoro');
		await expect.element(workField()).toHaveValue('25');
		await expect
			.element(page.getByRole('form', { name: 'Réglages' }))
			.toHaveAccessibleDescription(/^Enregistrés automatiquement/);
		await expect.element(page.getByRole('button', { name: 'Appliquer' })).not.toBeInTheDocument();
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
		const { onapply, unmount } = await show();
		await userEvent.type(labelField(), 'Focus');
		await unmount();
		expect(onapply).toHaveBeenCalledExactlyOnceWith({
			label: 'Focus',
			settings: INSTANCE.settings
		});
	});

	it('saves a text still being typed when the page is left', async () => {
		const { onapply } = await show();
		await userEvent.type(labelField(), 'Focus');
		window.dispatchEvent(new Event('pagehide'));
		expect(onapply).toHaveBeenCalledExactlyOnceWith({
			label: 'Focus',
			settings: INSTANCE.settings
		});
	});

	it('gives back the saved value on Escape, saving nothing', async () => {
		const { onapply } = await show();
		await userEvent.type(labelField(), 'Focus{Escape}');
		await expect.element(labelField()).toHaveValue('');
		await expect.element(revertedStatus()).toBeVisible();
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
		await expect
			.element(
				page.getByText(/^Non enregistré tant que ce n’est pas corrigé : Durée du travail\.$/)
			)
			.toBeVisible();

		await page.getByRole('checkbox', { name: 'Son' }).click();
		expect(onapply).toHaveBeenCalledExactlyOnceWith({
			label: null,
			settings: { workMinutes: 25, sound: true }
		});
		await expect.element(workField()).toHaveValue('0');

		await userEvent.type(workField(), '{Backspace}45{Escape}');
		await expect.element(workField()).toHaveValue('25');
		await expect.element(workField()).not.toHaveAttribute('aria-invalid');
		await expect.element(revertedStatus()).toBeVisible();
	});

	it('shows no error at a pause in the typing, but takes a corrected one away', async () => {
		const { onapply } = await show();
		await userEvent.type(workField(), '{Backspace}{Backspace}0');
		await expect.poll(() => onapply.mock.calls.length, { timeout: AFTER_TYPING_PAUSE_MS }).toBe(0);
		await expect.element(workField()).not.toHaveAttribute('aria-invalid');
		await userEvent.tab();
		await expect.element(workField()).toHaveAttribute('aria-invalid', 'true');

		await userEvent.type(workField(), '{Backspace}9');
		await expect
			.poll(() => onapply.mock.calls, { timeout: AFTER_TYPING_PAUSE_MS })
			.toEqual([[{ label: null, settings: { workMinutes: 9, sound: false } }]]);
		await expect.element(workField()).not.toHaveAttribute('aria-invalid');
		await expect.element(invalidStatus()).not.toBeInTheDocument();
	});

	it('keeps what was typed and says why when the manager refuses it', async () => {
		const { onapply } = await show({ apply: () => Promise.resolve(REFUSAL) });
		await userEvent.type(labelField(), 'Focus{Enter}');
		await expect.element(refusalAlert()).toBeVisible();
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

	it('saves the latest values after a refusal, and then says they are saved', async () => {
		const answers: ((outcome: EditOutcome) => void)[] = [];
		const { onapply } = await show({
			apply: () => new Promise((resolve) => answers.push(resolve))
		});
		await userEvent.type(labelField(), 'A{Enter}B{Enter}');
		answers[0]?.(REFUSAL);
		await expect.poll(() => onapply.mock.calls.length).toBe(2);
		expect(onapply).toHaveBeenLastCalledWith({ label: 'AB', settings: INSTANCE.settings });
		answers[1]?.({ saved: true });
		await expect.element(savedStatus()).toBeVisible();
		await expect.element(refusalAlert()).not.toBeInTheDocument();
	});

	it('keeps what was typed when the save that follows another is refused', async () => {
		const answers: ((outcome: EditOutcome) => void)[] = [];
		await show({ apply: () => new Promise((resolve) => answers.push(resolve)) });
		await userEvent.type(labelField(), 'A{Enter}B{Enter}');
		answers[0]?.({ saved: true });
		await expect.poll(() => answers.length).toBe(2);
		answers[1]?.(REFUSAL);
		await expect.element(refusalAlert()).toBeVisible();
		await expect.element(labelField()).toHaveValue('AB');
		await expect.element(savedStatus()).not.toBeInTheDocument();
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

	it('leaves a file being edited open when another setting changes elsewhere', async () => {
		const { screen } = await show({ module: DOCKER, instance: DOCKER_INSTANCE });
		await page.getByRole('button', { name: 'Ajouter un fichier…' }).click();
		await userEvent.type(page.getByLabelText('Fichier'), '/srv/blog/compose.yaml');
		await screen.rerender({ instance: { ...DOCKER_INSTANCE, label: 'Ailleurs' } });
		await expect.element(labelField()).toHaveValue('Ailleurs');
		await expect.element(page.getByLabelText('Fichier')).toHaveValue('/srv/blog/compose.yaml');
		await expect.element(page.getByLabelText('Fichier')).toHaveFocus();
	});

	it('shows a list of files changed elsewhere', async () => {
		const { screen } = await show({ module: DOCKER, instance: DOCKER_INSTANCE });
		await screen.rerender({
			instance: {
				...DOCKER_INSTANCE,
				settings: { files: [{ path: '/srv/blog/compose.yaml', name: 'Blog', icon: null }] }
			}
		});
		await expect.element(page.getByText('/srv/blog/compose.yaml')).toBeVisible();
		await expect.element(page.getByText('/srv/shop/compose.yaml')).not.toBeInTheDocument();
	});

	it('only offers the label for a module this version does not provide', async () => {
		await show({ module: null });
		await expect.element(page.getByText(/pas encore disponible dans cette version/)).toBeVisible();
		await expect.element(workField()).not.toBeInTheDocument();
	});
});
