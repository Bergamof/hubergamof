import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { HapticSettings } from '#lib/generated/index.js';
import HapticsPanel from './HapticsPanel.svelte';
import { CATALOG } from './mouse-fixtures.js';

async function renderPanel(haptics: HapticSettings = { level: 'medium', events: {} }) {
	const handlers = { onlevel: vi.fn(), onevent: vi.fn() };
	await render(HapticsPanel, { haptics, catalog: CATALOG, ...handlers });
	return handlers;
}

const WAVEFORM = page.getByLabelText('Vibration de : Pomodoro, Fin d’étape');

describe('HapticsPanel', () => {
	it('shows the strength and lists the events of the modules that have some', async () => {
		await renderPanel();
		await expect.element(page.getByRole('radio', { name: 'Moyenne' })).toBeChecked();
		await expect.element(page.getByRole('group', { name: 'Pomodoro' })).toBeVisible();
		expect(document.body.textContent).not.toContain('Docker');
	});

	it('sends the strength chosen', async () => {
		const { onlevel } = await renderPanel();
		await page.getByRole('radio', { name: 'Aucune vibration' }).click();
		expect(onlevel).toHaveBeenCalledExactlyOnceWith('off');
	});

	it('plays an event the way its module made it until told otherwise', async () => {
		await renderPanel();
		await expect.element(page.getByRole('switch', { name: 'Fin d’étape' })).toBeChecked();
		await expect.element(WAVEFORM).toHaveValue('completed');
	});

	it('shows what was chosen for an event', async () => {
		await renderPanel({
			level: 'low',
			events: { 'pomodoro.step-end': { enabled: false, waveform: 'knock' } }
		});
		await expect.element(page.getByRole('switch', { name: 'Fin d’étape' })).not.toBeChecked();
		await expect.element(WAVEFORM).toHaveValue('knock');
		await expect.element(WAVEFORM).toBeDisabled();
	});

	it('changes the waveform of an event', async () => {
		const { onevent } = await renderPanel();
		await WAVEFORM.selectOptions('ringing');
		expect(onevent).toHaveBeenCalledExactlyOnceWith('pomodoro.step-end', {
			enabled: true,
			waveform: 'ringing'
		});
	});

	it('switches an event off', async () => {
		const { onevent } = await renderPanel();
		await page.getByRole('switch', { name: 'Fin d’étape' }).click();
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
		await page.getByRole('switch', { name: 'Fin d’étape' }).click();
		expect(onevent).toHaveBeenCalledExactlyOnceWith('pomodoro.step-end', null);
	});

	it('says when the vibrations are off', async () => {
		await renderPanel({ level: 'off', events: {} });
		await expect.element(page.getByText(/Les vibrations sont coupées/)).toBeVisible();
	});

	it('says when no module signals anything', async () => {
		const handlers = { onlevel: vi.fn(), onevent: vi.fn() };
		await render(HapticsPanel, {
			haptics: { level: 'medium', events: {} },
			catalog: [],
			...handlers
		});
		await expect.element(page.getByText(/Aucun module ne signale/)).toBeVisible();
	});
});
