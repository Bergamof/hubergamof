import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import IntegerField from './IntegerField.svelte';

async function show(value = 5) {
	const onchange = vi.fn();
	await render(IntegerField, {
		label: 'Délai',
		value,
		min: 0,
		max: 300,
		unit: 's',
		hint: 'Entre 0 et 300 s.',
		error: 'Saisissez un nombre entier entre 0 et 300.',
		onchange
	});
	return { onchange, field: page.getByLabelText('Délai') };
}

describe('IntegerField', () => {
	it('shows the value, the unit and the hint', async () => {
		const { field } = await show(30);
		await expect.element(field).toHaveValue('30');
		await expect.element(page.getByText('Entre 0 et 300 s.')).toBeVisible();
		await expect.element(field).not.toHaveAttribute('aria-invalid');
	});

	it('sends each valid whole number within bounds', async () => {
		const { onchange, field } = await show();
		await field.fill('42');
		expect(onchange).toHaveBeenLastCalledWith(42);
		await field.fill('0');
		expect(onchange).toHaveBeenLastCalledWith(0);
		await field.fill('300');
		expect(onchange).toHaveBeenLastCalledWith(300);
	});

	it.each(['301', '-1', '1.5', 'abc', ''])('sends nothing for %j', async (typed) => {
		const { onchange, field } = await show();
		await field.fill('7');
		onchange.mockClear();
		await field.fill(typed);
		expect(onchange).not.toHaveBeenCalled();
		await expect.element(field).toHaveAttribute('aria-invalid', 'true');
		await expect
			.element(page.getByText('Saisissez un nombre entier entre 0 et 300.'))
			.toBeVisible();
	});

	it('shows the saved value again when the field is left', async () => {
		const { field } = await show(12);
		await field.fill('999');
		await field.element().blur();
		await expect.element(field).toHaveValue('12');
		await expect.element(field).not.toHaveAttribute('aria-invalid');
	});
});
