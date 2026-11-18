import { describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';
import type { ManagerApi } from '#lib/api/manager-api.js';
import type { ModuleInstance } from '#lib/generated/index.js';
import KeyImagePicker from './KeyImagePicker.svelte';

const INSTANCE: ModuleInstance = {
	id: 'audio',
	kind: 'audio',
	label: null,
	image: null,
	settings: {}
};

const IMAGE = new File(['png'], 'logo.png', { type: 'image/png' });

async function renderPicker(
	options: {
		instance?: ModuleInstance;
		uploadKeyImage?: ManagerApi['uploadKeyImage'];
	} = {}
) {
	const onchange = vi.fn().mockResolvedValue({ saved: true });
	const { container } = await render(KeyImagePicker, {
		serial: 'DECK',
		instance: options.instance ?? INSTANCE,
		module: undefined,
		api: { uploadKeyImage: options.uploadKeyImage ?? vi.fn() } as unknown as ManagerApi,
		onchange
	});
	const input = container.querySelector('input[type="file"]') as HTMLInputElement;
	return { input, onchange };
}

function pick(input: HTMLInputElement, file: File) {
	const files = new DataTransfer();
	files.items.add(file);
	input.files = files.files;
	input.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('KeyImagePicker', () => {
	it('tells the module icon is used, with nothing to reset', async () => {
		await renderPicker();
		await expect.element(page.getByText('Icône du module.')).toBeVisible();
		await expect
			.element(page.getByRole('button', { name: 'Rétablir l’icône du module' }))
			.not.toBeInTheDocument();
	});

	it('saves an uploaded image on the key', async () => {
		const uploadKeyImage = vi.fn().mockResolvedValue({ ok: true, value: 'abc.png' });
		const { input, onchange } = await renderPicker({ uploadKeyImage });
		pick(input, IMAGE);
		await expect.poll(() => onchange.mock.calls.length).toBe(1);
		expect(uploadKeyImage).toHaveBeenCalledWith('DECK', IMAGE);
		expect(onchange).toHaveBeenCalledWith('abc.png');
	});

	it('shows why an image was refused and saves nothing', async () => {
		const uploadKeyImage = vi.fn().mockResolvedValue({
			ok: false,
			problem: { text: 'Image refusée.', detail: 'not an image' }
		});
		const { input, onchange } = await renderPicker({ uploadKeyImage });
		pick(input, IMAGE);
		await expect.element(page.getByText('Image refusée.')).toBeVisible();
		expect(onchange).not.toHaveBeenCalled();
	});

	it('resets a custom image to the module icon', async () => {
		const { onchange } = await renderPicker({ instance: { ...INSTANCE, image: 'abc.png' } });
		await expect.element(page.getByText('Image personnalisée.')).toBeVisible();
		await page.getByRole('button', { name: 'Rétablir l’icône du module' }).click();
		await expect.poll(() => onchange.mock.calls.length).toBe(1);
		expect(onchange).toHaveBeenCalledWith(null);
	});
});
