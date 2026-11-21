import { describe, expect, it, vi } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import ShortcutInput from './ShortcutInput.svelte';

const RECORD = 'Enregistrer en appuyant sur les touches';

async function renderInput(value = 'ctrl+shift+m') {
	const onchange = vi.fn();
	await render(ShortcutInput, { label: 'Raccourci', value, onchange });
	return onchange;
}

describe('ShortcutInput', () => {
	it('shows the saved shortcut and what it sends', async () => {
		await renderInput();
		await expect.element(page.getByLabelText('Raccourci')).toHaveValue('ctrl+shift+m');
		await expect.element(page.getByText(/Envoie Ctrl \+ Maj \+ M\./)).toBeVisible();
	});

	it('gives an example when there is no shortcut yet', async () => {
		await renderInput('');
		await expect.element(page.getByText(/Exemple : ctrl\+shift\+m\./)).toBeVisible();
	});

	it('sends a typed shortcut once the field is left, written as the manager reads it', async () => {
		const onchange = await renderInput('');
		const field = page.getByLabelText('Raccourci');
		await field.fill('Maj + Control + K');
		expect(onchange).not.toHaveBeenCalled();
		await userEvent.tab();
		expect(onchange).toHaveBeenCalledExactlyOnceWith('ctrl+shift+k');
	});

	it('does not send what it does not know, and says so', async () => {
		const onchange = await renderInput();
		await page.getByLabelText('Raccourci').fill('ctrl+banane');
		await userEvent.tab();
		expect(onchange).not.toHaveBeenCalled();
		await expect.element(page.getByText(/Touches inconnues/)).toBeVisible();
		await expect.element(page.getByLabelText('Raccourci')).toBeInvalid();
	});

	it('refuses a reserved combination', async () => {
		const onchange = await renderInput();
		await page.getByLabelText('Raccourci').fill('ctrl+alt+delete');
		await userEvent.tab();
		expect(onchange).not.toHaveBeenCalled();
		await expect.element(page.getByText(/réservée/)).toBeVisible();
	});

	it('does not send again a shortcut it already has', async () => {
		const onchange = await renderInput('ctrl+m');
		await page.getByLabelText('Raccourci').fill('Ctrl+M');
		await userEvent.tab();
		expect(onchange).not.toHaveBeenCalled();
	});

	it('records the keys pressed', async () => {
		const onchange = await renderInput('');
		await page.getByRole('button', { name: RECORD }).click();
		await expect
			.element(page.getByRole('button', { name: RECORD }))
			.toHaveAttribute('aria-pressed', 'true');
		await expect
			.element(page.getByText(/Enregistrement : appuyez sur la combinaison/))
			.toBeVisible();
		await userEvent.keyboard('{Control>}{Shift>}m{/Shift}{/Control}');
		expect(onchange).toHaveBeenCalledExactlyOnceWith('ctrl+shift+m');
		await expect
			.element(page.getByRole('button', { name: RECORD }))
			.toHaveAttribute('aria-pressed', 'false');
	});

	it('records a shortcut made of modifiers only, when they are released', async () => {
		const onchange = await renderInput('');
		await page.getByRole('button', { name: RECORD }).click();
		await userEvent.keyboard('{Control>}{Alt>}{/Alt}{/Control}');
		expect(onchange).toHaveBeenCalledExactlyOnceWith('ctrl+alt');
	});

	it('says when a recorded key cannot be sent', async () => {
		const onchange = await renderInput('');
		await page.getByRole('button', { name: RECORD }).click();
		await userEvent.keyboard('{CapsLock}');
		expect(onchange).not.toHaveBeenCalled();
		await expect.element(page.getByText(/ne peut pas être envoyée/)).toBeVisible();
	});

	it('stops recording when the button is pressed again or left', async () => {
		await renderInput('');
		const button = page.getByRole('button', { name: RECORD });
		await button.click();
		await button.click();
		await expect.element(button).toHaveAttribute('aria-pressed', 'false');
		await button.click();
		await userEvent.tab();
		await expect.element(button).toHaveAttribute('aria-pressed', 'false');
	});

	it('lets Tab leave the recording instead of trapping the keyboard', async () => {
		const onchange = await renderInput('');
		await page.getByRole('button', { name: RECORD }).click();
		await userEvent.tab();
		expect(onchange).not.toHaveBeenCalled();
	});

	it('does not stop recording at the release of Enter that started it', async () => {
		const onchange = await renderInput('');
		const button = page.getByRole('button', { name: RECORD });
		await button.element().focus();
		await userEvent.keyboard('{Enter}');
		await expect.element(button).toHaveAttribute('aria-pressed', 'true');
		await userEvent.keyboard('{Control>}k{/Control}');
		expect(onchange).toHaveBeenCalledExactlyOnceWith('ctrl+k');
	});

	it('refuses to record a reserved combination', async () => {
		const onchange = await renderInput('');
		await page.getByRole('button', { name: RECORD }).click();
		await userEvent.keyboard('{Control>}{Alt>}{Delete}{/Alt}{/Control}');
		expect(onchange).not.toHaveBeenCalled();
		await expect.element(page.getByText(/réservée/)).toBeVisible();
	});

	it('announces what was saved', async () => {
		await renderInput('');
		await page.getByLabelText('Raccourci').fill('ctrl+m');
		await userEvent.tab();
		await expect.element(page.getByRole('status')).toHaveTextContent('Raccourci Ctrl + M.');
	});

	it('tells what the keys are on the keyboard of the person when it is not the American one', async () => {
		const onchange = vi.fn();
		const azerty = new Map([['KeyA', 'q']]);
		await render(ShortcutInput, { label: 'Raccourci', value: 'ctrl+a', onchange, layout: azerty });
		await expect.element(page.getByText(/s’écrivent Ctrl \+ Q/)).toBeVisible();
	});

	it('says nothing more when the keyboard is the American one', async () => {
		const onchange = vi.fn();
		const american = new Map([['KeyA', 'a']]);
		await render(ShortcutInput, {
			label: 'Raccourci',
			value: 'ctrl+a',
			onchange,
			layout: american
		});
		expect(document.body.textContent).not.toContain('s’écrivent');
	});

	it('warns that the browser keeps some combinations, which are written in the field', async () => {
		await renderInput('');
		await expect
			.element(page.getByText(/Le navigateur garde certaines combinaisons/))
			.toBeVisible();
	});

	it('learns the keyboard from the keys pressed, and tells what the shortcut is on it', async () => {
		localStorage.removeItem('hubergamof.keyboard-layout');
		const onchange = vi.fn();
		await render(ShortcutInput, { label: 'Raccourci', value: 'ctrl+a', onchange, locale: 'it-IT' });
		await expect.element(page.getByText(/appuyez une fois dessus/)).toBeVisible();
		// On an AZERTY keyboard, the key at the American A writes q.
		window.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyA', key: 'q' }));
		await expect.element(page.getByText(/s’écrivent Ctrl \+ Q/)).toBeVisible();
		expect(document.body.textContent).not.toContain('appuyez une fois dessus');
		localStorage.removeItem('hubergamof.keyboard-layout');
	});

	it('guesses the keyboard from the language while it is not known, and says it is a guess', async () => {
		await render(ShortcutInput, {
			label: 'Raccourci',
			value: 'ctrl+a',
			onchange: vi.fn(),
			layout: new Map(),
			locale: 'fr-FR'
		});
		await expect
			.element(
				page.getByText(
					/Avec le clavier de votre langue \(AZERTY\), ces touches s’écrivent Ctrl \+ Q/
				)
			)
			.toBeVisible();
	});

	it('does not ask for key presses to a person whose language goes with the American keyboard', async () => {
		await render(ShortcutInput, {
			label: 'Raccourci',
			value: 'ctrl+a',
			onchange: vi.fn(),
			layout: new Map(),
			locale: 'en-US'
		});
		expect(document.body.textContent).not.toContain('appuyez une fois dessus');
		expect(document.body.textContent).not.toContain('Avec le clavier de votre langue');
	});
});
