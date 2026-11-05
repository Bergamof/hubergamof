import { describe, expect, it } from 'vitest';
import {
	MAX_ARCHIVE_BYTES,
	MAX_IMAGE_BYTES,
	ManagerApi,
	exportFileName,
	exportUrl,
	keyImageUrl,
	mebibytes,
	type Fetch
} from './manager-api.js';

interface Call {
	input: string;
	init: RequestInit | undefined;
}

function fakeFetch(answer: () => Response | Promise<Response>) {
	const calls: Call[] = [];
	const fetchImpl: Fetch = async (input, init) => {
		calls.push({ input, init });
		return answer();
	};
	return { calls, api: new ManagerApi(fetchImpl) };
}

function json(status: number, body: unknown): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

const png = new File([new Uint8Array([137, 80, 78, 71])], 'logo.png', { type: 'image/png' });

describe('URLs', () => {
	it('escape the serial and the image name', () => {
		expect(keyImageUrl('A B', 'x.png')).toBe('/api/devices/A%20B/images/x.png');
		expect(exportUrl('A/B')).toBe('/api/devices/A%2FB/export');
	});
});

describe('exportFileName', () => {
	it('names the archive after the device and the local day', () => {
		expect(exportFileName('DECK', new Date(2026, 0, 5, 23, 59))).toBe(
			'bergadeck-DECK-2026-01-05.zip'
		);
	});
});

describe('mebibytes', () => {
	it('shows the limits as the manager defines them', () => {
		expect(mebibytes(MAX_IMAGE_BYTES)).toBe('4 Mio');
		expect(mebibytes(MAX_ARCHIVE_BYTES)).toBe('32 Mio');
	});
});

describe('uploadKeyImage', () => {
	it('sends a file of unknown type for the manager to identify', async () => {
		const { calls, api } = fakeFetch(() => json(200, { name: 'abc.png' }));
		const untyped = new File([new Uint8Array([137, 80])], 'logo', { type: '' });
		await expect(api.uploadKeyImage('DECK', untyped)).resolves.toEqual({
			ok: true,
			value: 'abc.png'
		});
		expect(calls[0].init).toMatchObject({
			headers: { 'content-type': 'application/octet-stream' }
		});
	});

	it('reports a successful answer it cannot read', async () => {
		const { api } = fakeFetch(() => new Response('not json', { status: 200 }));
		await expect(api.uploadKeyImage('DECK', png)).resolves.toMatchObject({
			ok: false,
			problem: { text: expect.stringMatching(/inattendue/) }
		});
	});

	it('posts the image with its type and resolves its stored name', async () => {
		const { calls, api } = fakeFetch(() => json(200, { name: 'abc.png' }));
		await expect(api.uploadKeyImage('DECK', png)).resolves.toEqual({ ok: true, value: 'abc.png' });
		expect(calls).toHaveLength(1);
		expect(calls[0].input).toBe('/api/devices/DECK/images');
		expect(calls[0].init).toMatchObject({
			method: 'POST',
			headers: { 'content-type': 'image/png' },
			body: png
		});
	});

	it('refuses other formats and oversized files without sending them', async () => {
		const { calls, api } = fakeFetch(() => json(200, { name: 'never' }));
		const gif = new File(['GIF89a'], 'a.gif', { type: 'image/gif' });
		const huge = new File([new Uint8Array(MAX_IMAGE_BYTES + 1)], 'big.png', { type: 'image/png' });
		await expect(api.uploadKeyImage('DECK', gif)).resolves.toMatchObject({
			ok: false,
			problem: { text: expect.stringMatching(/PNG, JPEG ou SVG/) }
		});
		await expect(api.uploadKeyImage('DECK', huge)).resolves.toMatchObject({
			ok: false,
			problem: { text: expect.stringMatching(/4 Mio/) }
		});
		expect(calls).toEqual([]);
	});

	it('explains the refusals of the manager in French, with its details', async () => {
		const { api } = fakeFetch(() => json(400, { code: 'invalidImage', message: 'bad png' }));
		await expect(api.uploadKeyImage('DECK', png)).resolves.toEqual({
			ok: false,
			problem: {
				text: 'Image refusée : elle doit être un PNG, un JPEG ou un SVG lisible.',
				detail: 'bad png'
			}
		});
	});

	it('reports a body limit, an answer that is not the API and an unreachable manager', async () => {
		const tooLarge = fakeFetch(() => new Response('length limit exceeded', { status: 413 }));
		await expect(tooLarge.api.uploadKeyImage('DECK', png)).resolves.toMatchObject({
			problem: { text: expect.stringMatching(/4 Mio/) }
		});
		const proxy = fakeFetch(() => new Response('Bad Gateway', { status: 502 }));
		await expect(proxy.api.uploadKeyImage('DECK', png)).resolves.toMatchObject({
			problem: { detail: 'HTTP 502' }
		});
		const down = fakeFetch(() => Promise.reject(new TypeError('Failed to fetch')));
		await expect(down.api.uploadKeyImage('DECK', png)).resolves.toMatchObject({
			problem: { text: expect.stringMatching(/injoignable/) }
		});
	});
});

describe('importArchive', () => {
	it('posts the archive', async () => {
		const saved = { serial: 'DECK', revision: 4 };
		const { calls, api } = fakeFetch(() => json(200, saved));
		const archive = new File(['PK'], 'export.zip', { type: '' });
		await expect(api.importArchive('DECK', archive)).resolves.toEqual({
			ok: true,
			value: undefined
		});
		expect(calls[0]).toMatchObject({
			input: '/api/devices/DECK/import',
			init: { headers: { 'content-type': 'application/zip' } }
		});
	});

	it('explains a configuration changed meanwhile', async () => {
		const { api } = fakeFetch(() => json(409, { code: 'staleRevision', message: 'stale' }));
		await expect(api.importArchive('DECK', new File(['x'], 'a.zip'))).resolves.toMatchObject({
			problem: { text: expect.stringMatching(/modifiée ailleurs/) }
		});
	});

	it('explains a refused archive and refuses an oversized one locally', async () => {
		const { calls, api } = fakeFetch(() =>
			json(400, { code: 'invalidArchive', message: 'no config.json' })
		);
		await expect(api.importArchive('DECK', new File(['x'], 'a.zip'))).resolves.toMatchObject({
			problem: { text: expect.stringMatching(/export Bergadeck/), detail: 'no config.json' }
		});
		const huge = new File([new Uint8Array(MAX_ARCHIVE_BYTES + 1)], 'big.zip');
		await expect(api.importArchive('DECK', huge)).resolves.toMatchObject({
			problem: { text: expect.stringMatching(/32 Mio/) }
		});
		expect(calls).toHaveLength(1);
	});
});
