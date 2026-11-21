import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { CatalogModule, HapticSettings } from '#lib/generated/index.js';
import HapticsPanel from './HapticsPanel.svelte';
import { CATALOG, catalogModule } from './mouse-fixtures.js';

const TEAMS = catalogModule(
	'teams',
	'Teams',
	[],
	[
		['message', 'Nouveau message'],
		['call', 'Appel entrant']
	]
);

async function renderPanel(
	haptics: HapticSettings = { level: 'medium', events: {} },
	catalog: CatalogModule[] = CATALOG,
	saved = true
) {
	const handlers = {
		onlevel: vi.fn().mockResolvedValue(saved),
		onevent: vi.fn(),
		onplay: vi.fn()
	};
	await render(HapticsPanel, { haptics, catalog, ...handlers });
	return handlers;
}

const WAVEFORM = page.getByLabelText('Vibration de : Pomodoro, Fin d’étape');
const STATUS = page.getByRole('status');

describe('HapticsPanel', () => {
	it('shows the strength and lists the events of the modules that have some', async () => {
		await renderPanel();
		await expect
			.element(page.getByRole('radiogroup', { name: 'Force des vibrations' }))
			.toBeVisible();
		await expect.element(page.getByRole('radio', { name: 'Moyenne' })).toBeChecked();
		await expect.element(page.getByRole('heading', { name: /Pomodoro/ })).toBeVisible();
		expect(document.body.textContent).not.toContain('Docker');
		await expect.element(STATUS).toHaveTextContent('Choisissez une onde pour la sentir.');
	});

	it('plays the new strength once the manager has taken it', async () => {
		const { onlevel, onplay } = await renderPanel();
		await page.getByRole('radio', { name: 'Forte' }).click();
		expect(onlevel).toHaveBeenCalledExactlyOnceWith('high');
		await vi.waitFor(() => expect(onplay).toHaveBeenCalledExactlyOnceWith('sharpStateChange'));
		await expect.element(page.getByText(/Joué sur la souris/)).toBeVisible();
	});

	it('plays nothing when the manager refused the strength', async () => {
		const { onplay } = await renderPanel(undefined, undefined, false);
		await page.getByRole('radio', { name: 'Forte' }).click();
		await new Promise((resolve) => setTimeout(resolve, 50));
		expect(onplay).not.toHaveBeenCalled();
	});

	it('says that nothing is played when the vibrations are off', async () => {
		await renderPanel({ level: 'off', events: {} });
		await page.getByRole('button', { name: 'Jouer : Fin d’étape' }).click();
		await expect.element(page.getByText(/Retour haptique désactivé/)).toBeVisible();
	});

	it('plays an event the way its module made it until told otherwise', async () => {
		await renderPanel();
		await expect.element(page.getByRole('switch', { name: 'Activer : Fin d’étape' })).toBeChecked();
		await expect.element(WAVEFORM).toHaveValue('completed');
	});

	it('shows what was chosen for an event', async () => {
		await renderPanel({
			level: 'low',
			events: { 'pomodoro.step-end': { enabled: false, waveform: 'knock' } }
		});
		await expect
			.element(page.getByRole('switch', { name: 'Activer : Fin d’étape' }))
			.not.toBeChecked();
		await expect.element(WAVEFORM).toHaveValue('knock');
		await expect.element(WAVEFORM).toBeDisabled();
		await expect.element(page.getByRole('button', { name: 'Jouer : Fin d’étape' })).toBeDisabled();
	});

	it('changes the waveform of an event and plays it at once', async () => {
		const { onevent, onplay } = await renderPanel();
		await WAVEFORM.selectOptions('ringing');
		expect(onevent).toHaveBeenCalledExactlyOnceWith('pomodoro.step-end', {
			enabled: true,
			waveform: 'ringing'
		});
		expect(onplay).toHaveBeenCalledExactlyOnceWith('ringing');
		await expect.element(STATUS).toHaveTextContent('Joué sur la souris : Sonnerie');
	});

	it('plays an event on demand', async () => {
		const { onplay } = await renderPanel();
		await page.getByRole('button', { name: 'Jouer : Fin d’étape' }).click();
		expect(onplay).toHaveBeenCalledExactlyOnceWith('completed');
	});

	it('switches an event off', async () => {
		const { onevent } = await renderPanel();
		await page.getByRole('switch', { name: 'Activer : Fin d’étape' }).click();
		expect(onevent).toHaveBeenCalledExactlyOnceWith('pomodoro.step-end', {
			enabled: false,
			waveform: 'completed'
		});
	});

	it('forgets the choice of an event brought back to the default', async () => {
		const { onevent } = await renderPanel({
			level: 'medium',
			events: { 'pomodoro.step-end': { enabled: false, waveform: 'completed' } }
		});
		await page.getByRole('switch', { name: 'Activer : Fin d’étape' }).click();
		expect(onevent).toHaveBeenCalledExactlyOnceWith('pomodoro.step-end', null);
	});

	it('switches a whole module on or off, and shows it half way when it is', async () => {
		const { onevent } = await renderPanel(
			{ level: 'medium', events: { 'teams.call': { enabled: false, waveform: 'ringing' } } },
			[TEAMS]
		);
		const all = page.getByRole('switch', {
			name: 'Activer ou désactiver tous les retours de Teams'
		});
		expect((all.element() as HTMLInputElement).indeterminate).toBe(true);
		await all.click();
		// Only the event that was off changes, and keeps the waveform chosen for it.
		expect(onevent).toHaveBeenCalledExactlyOnceWith('teams.call', {
			enabled: true,
			waveform: 'ringing'
		});
	});

	it('switches off every event of a module that has them all on', async () => {
		const { onevent } = await renderPanel(undefined, [TEAMS]);
		const all = page.getByRole('switch', {
			name: 'Activer ou désactiver tous les retours de Teams'
		});
		await expect.element(all).toBeChecked();
		await all.click();
		expect(onevent).toHaveBeenCalledTimes(2);
		expect(onevent).toHaveBeenCalledWith(
			'teams.message',
			expect.objectContaining({ enabled: false })
		);
		expect(onevent).toHaveBeenCalledWith('teams.call', expect.objectContaining({ enabled: false }));
	});

	it('says when no module signals anything', async () => {
		await renderPanel(undefined, []);
		await expect.element(page.getByText(/Aucun module ne signale/)).toBeVisible();
	});
});
