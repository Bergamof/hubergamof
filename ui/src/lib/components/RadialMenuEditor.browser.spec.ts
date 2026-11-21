import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { RadialMenuConfig } from '#lib/generated/index.js';
import { AUDIO, POMODORO, catalogModule, mouseConfig, radialMenu } from './mouse-fixtures.js';
import RadialMenuEditor from './RadialMenuEditor.svelte';

const TEAMS = catalogModule('teams', 'Teams', [['mute', 'Couper le micro']]);
const INTELLIJ = catalogModule('intellij', 'IntelliJ', [['open', 'Ouvrir un projet']]);
const DOCKER = catalogModule('docker', 'Docker', [['restart', 'Redémarrer']]);
const CATALOG = [POMODORO, AUDIO, TEAMS, INTELLIJ, DOCKER];

/** Applies the edits to the menu the way the state does, to read what they give. */
async function renderMenu(menu: RadialMenuConfig = radialMenu()) {
	const onedit = vi.fn();
	await render(RadialMenuEditor, { menu, catalog: CATALOG, onedit });
	const last = () => onedit.mock.lastCall![0](menu) as RadialMenuConfig;
	return { onedit, last };
}

describe('RadialMenuEditor', () => {
	it('lists the entries with what a click on each does', async () => {
		await renderMenu();
		await expect
			.element(page.getByText(/Entre 3 et 8 entrées affichées \(4 actuellement\)/))
			.toBeVisible();
		await expect
			.element(page.getByLabelText('Action du clic sur : Pomodoro'))
			.toHaveValue('toggle');
	});

	it('changes what a click does, or closes the menu', async () => {
		const { last } = await renderMenu();
		await page.getByLabelText('Action du clic sur : Pomodoro').selectOptions('skip');
		expect(last().entries[0].primary).toEqual({ kind: 'action', action: 'skip' });
		await page.getByLabelText('Action du clic sur : Pomodoro').selectOptions('Fermer le menu');
		expect(last().entries[0].primary).toEqual({ kind: 'close' });
	});

	it('keeps showing an action that no longer exists', async () => {
		await renderMenu(
			radialMenu({
				entries: radialMenu().entries.map((entry) =>
					entry.module === 'pomodoro'
						? { ...entry, primary: { kind: 'action', action: 'old' } }
						: entry
				)
			})
		);
		await expect.element(page.getByRole('option', { name: 'old (absente)' })).toBeInTheDocument();
	});

	it('shows or hides an entry', async () => {
		const { last } = await renderMenu();
		await page.getByRole('switch', { name: 'Pomodoro' }).click();
		expect(last().entries[0].enabled).toBe(false);
	});

	it('moves an entry, except past the ends of the list', async () => {
		const { last } = await renderMenu();
		await expect
			.element(page.getByRole('button', { name: 'Monter : Pomodoro' }))
			.toHaveAttribute('aria-disabled', 'true');
		await expect
			.element(page.getByRole('button', { name: 'Descendre : IntelliJ' }))
			.toHaveAttribute('aria-disabled', 'true');
		await page.getByRole('button', { name: 'Descendre : Pomodoro' }).click();
		expect(last().entries.map((entry) => entry.module)).toEqual([
			'audio',
			'pomodoro',
			'teams',
			'intellij'
		]);
		await page.getByRole('button', { name: 'Monter : Audio' }).click();
		expect(last().entries[0].module).toBe('audio');
	});

	it('removes an entry', async () => {
		const { last } = await renderMenu(
			radialMenu({
				entries: [
					...radialMenu().entries,
					{ module: 'x', enabled: false, primary: { kind: 'close' } }
				]
			})
		);
		await page.getByRole('button', { name: 'Retirer : x' }).click();
		expect(last().entries.map((entry) => entry.module)).not.toContain('x');
	});

	it('does not let the menu fall under the fewest entries', async () => {
		await renderMenu(radialMenu({ entries: radialMenu().entries.slice(0, 3) }));
		await expect.element(page.getByRole('switch', { name: 'Pomodoro' })).toBeDisabled();
		await expect
			.element(page.getByRole('button', { name: 'Retirer : Pomodoro' }))
			.toHaveAttribute('aria-disabled', 'true');
		await expect.element(page.getByText(/Minimum atteint/)).toBeVisible();
	});

	it('does not let the menu go over the most entries', async () => {
		const modules = Array.from({ length: 8 }, (_, index) => `m${index}`);
		await renderMenu(
			radialMenu({
				entries: [
					...modules.map((module) => ({
						module,
						enabled: true,
						primary: { kind: 'close' as const }
					})),
					{ module: 'extra', enabled: false, primary: { kind: 'close' as const } }
				]
			})
		);
		await expect.element(page.getByRole('switch', { name: 'extra' })).toBeDisabled();
		await expect
			.element(page.getByRole('button', { name: 'Ajouter' }))
			.toHaveAttribute('aria-disabled', 'true');
		await expect.element(page.getByText(/Maximum atteint/)).toBeVisible();
	});

	it('adds a module that offers actions and is not listed yet', async () => {
		const { last } = await renderMenu();
		const choice = page.getByLabelText('Ajouter un module');
		await expect.element(choice).toHaveValue('docker');
		await page.getByRole('button', { name: 'Ajouter' }).click();
		expect(last().entries.at(-1)).toEqual({
			module: 'docker',
			enabled: true,
			primary: { kind: 'action', action: 'restart' }
		});
	});

	it('offers nothing to add when every module is listed', async () => {
		await renderMenu(
			radialMenu({
				entries: ['pomodoro', 'audio', 'teams', 'intellij', 'docker'].map((module) => ({
					module,
					enabled: true,
					primary: { kind: 'close' as const }
				}))
			})
		);
		expect(document.body.textContent).not.toContain('Ajouter un module');
	});

	it('changes the hold time, and rejects a value out of bounds', async () => {
		const { onedit, last } = await renderMenu();
		await page.getByLabelText('Durée du maintien').fill('400');
		expect(last().holdMs).toBe(400);
		onedit.mockClear();
		await page.getByLabelText('Durée du maintien').fill('900');
		expect(onedit).not.toHaveBeenCalled();
		await expect.element(page.getByRole('alert')).toBeVisible();
	});

	it('switches the animation and the vibrations', async () => {
		const { last } = await renderMenu();
		await page.getByRole('switch', { name: /Animer/ }).click();
		expect(last().animate).toBe(false);
		await page.getByRole('switch', { name: /Vibrations du menu/ }).click();
		expect(last().haptics).toBe(false);
		expect(mouseConfig().serial).toBe('mx-1');
	});

	it('adds the module chosen when several are not listed', async () => {
		const { last } = await renderMenu(
			radialMenu({ entries: radialMenu().entries.filter((entry) => entry.module !== 'audio') })
		);
		await page.getByLabelText('Ajouter un module').selectOptions('audio');
		await page.getByRole('button', { name: 'Ajouter' }).click();
		expect(last().entries.at(-1)?.module).toBe('audio');
	});

	it('names a module the catalog no longer has by its identifier', async () => {
		await renderMenu(
			radialMenu({
				entries: [
					...radialMenu().entries,
					{ module: 'gone', enabled: false, primary: { kind: 'close' } }
				]
			})
		);
		await expect.element(page.getByRole('button', { name: 'Monter : gone' })).toBeVisible();
		await expect.element(page.getByLabelText('Action du clic sur : gone')).toHaveValue('');
	});

	it('announces a move and keeps the button, inactive at the end of the list', async () => {
		const { onedit } = await renderMenu();
		// Playwright treats aria-disabled as disabled: forced, like a person pressing it.
		await page.getByRole('button', { name: 'Monter : Pomodoro' }).click({ force: true });
		expect(onedit).not.toHaveBeenCalled();
		await page.getByRole('button', { name: 'Descendre : Pomodoro' }).click();
		await expect.element(page.getByText('Pomodoro : position 2 sur 4.')).toBeInTheDocument();
	});

	it('does not remove the entry that keeps the menu at its minimum', async () => {
		const { onedit } = await renderMenu(radialMenu({ entries: radialMenu().entries.slice(0, 3) }));
		await page.getByRole('button', { name: 'Retirer : Pomodoro' }).click({ force: true });
		expect(onedit).not.toHaveBeenCalled();
	});

	it('moves the focus to the list once an entry is removed', async () => {
		const { onedit } = await renderMenu(
			radialMenu({
				entries: [
					...radialMenu().entries,
					{ module: 'x', enabled: false, primary: { kind: 'close' } }
				]
			})
		);
		await page.getByRole('button', { name: 'Retirer : x' }).click();
		expect(onedit).toHaveBeenCalledOnce();
		await expect.element(page.getByRole('list', { name: 'Entrées du menu' })).toHaveFocus();
	});
});
