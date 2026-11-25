import { describe, expect, it } from 'vitest';
import {
	MAX_ARCHIVE_BYTES,
	MAX_IMAGE_BYTES,
	ManagerApi,
	backupFileName,
	backupUrl,
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
	});

	it('give the whole backup, or the kinds of document asked for', () => {
		expect(backupUrl()).toBe('/api/backup/export');
		expect(backupUrl(['devices', 'radial-menu'])).toBe(
			'/api/backup/export?sections=devices%2Cradial-menu'
		);
	});
});

describe('backupFileName', () => {
	it('names the backup after the local day', () => {
		expect(backupFileName(new Date(2026, 0, 5, 23, 59))).toBe('hubergamof-2026-01-05.zip');
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
			code: 'invalidImage',
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

describe('chooseFile', () => {
	it('names the setting to the file chooser and resolves the chosen path', async () => {
		const { calls, api } = fakeFetch(() => json(200, { path: '/srv/shop/compose.yaml' }));
		await expect(api.chooseFile('docker', 'composeFiles')).resolves.toEqual({
			ok: true,
			value: '/srv/shop/compose.yaml'
		});
		expect(calls[0]).toMatchObject({ input: '/api/file-chooser', init: { method: 'POST' } });
		const body = (calls[0]?.init as { body: Blob }).body;
		expect(JSON.parse(await body.text())).toEqual({ module: 'docker', setting: 'composeFiles' });
	});

	it('resolves null when the user closed the dialog', async () => {
		const { api } = fakeFetch(() => json(200, { path: null }));
		await expect(api.chooseFile('docker', 'composeFiles')).resolves.toEqual({
			ok: true,
			value: null
		});
	});

	it('says whether there is no dialog on this desktop or one is already open', async () => {
		const missing = fakeFetch(() =>
			json(503, { code: 'unavailable', message: 'no file chooser is available on this desktop' })
		);
		await expect(missing.api.chooseFile('docker', 'composeFiles')).resolves.toMatchObject({
			ok: false,
			code: 'unavailable',
			problem: { text: expect.stringMatching(/saisissez le chemin/), detail: null }
		});
		const busy = fakeFetch(() => json(409, { code: 'busy', message: 'a file chooser is open' }));
		await expect(busy.api.chooseFile('docker', 'composeFiles')).resolves.toMatchObject({
			ok: false,
			code: 'busy',
			problem: { text: expect.stringMatching(/déjà ouvert/) }
		});
	});
});

const zip = new File(['PK'], 'backup.zip', { type: '' });
const preview = { sections: [], kept: [] };
const report = { snapshot: '1-0', sections: [] };

describe('backup routes', () => {
	it('fetches the backup as a blob, refusals shown rather than saved', async () => {
		const ok = fakeFetch(() => new Response('PK', { status: 200 }));
		const exported = await ok.api.exportBackup(['git', 'ssh']);
		expect(exported.ok && (await exported.value.text())).toBe('PK');
		expect(ok.calls[0]).toMatchObject({
			input: '/api/backup/export?sections=git%2Cssh',
			init: { method: 'GET' }
		});
		const locked = fakeFetch(() => json(423, { code: 'locked', message: 'locked' }));
		await expect(locked.api.exportBackup()).resolves.toMatchObject({ ok: false, code: 'locked' });
	});

	it('posts the file for a preview and reads what it would do', async () => {
		const { calls, api } = fakeFetch(() => json(200, preview));
		await expect(api.previewBackup(zip)).resolves.toEqual({ ok: true, value: preview });
		expect(calls[0].input).toBe('/api/backup/preview');
		expect(calls[0].init).toMatchObject({
			method: 'POST',
			headers: { 'content-type': 'application/zip' },
			body: zip
		});
	});

	it('imports the sections chosen, with the devices that receive another’s', async () => {
		const { calls, api } = fakeFetch(() => json(200, report));
		const request = { sections: ['devices/OLD', 'git'], devices: { OLD: 'NEW' } };
		await expect(api.importBackup(zip, request)).resolves.toEqual({ ok: true, value: report });
		expect(calls[0].input).toBe('/api/backup/import?sections=devices%2FOLD%2Cgit&map=OLD%3ANEW');
		const plain = fakeFetch(() => json(200, report));
		await plain.api.importBackup(zip, { sections: ['git'], devices: {} });
		expect(plain.calls[0].input).toBe('/api/backup/import?sections=git');
	});

	it('refuses an oversized file without sending it', async () => {
		const { calls, api } = fakeFetch(() => json(200, report));
		const huge = new Blob([new Uint8Array(MAX_ARCHIVE_BYTES + 1)]);
		await expect(api.previewBackup(huge)).resolves.toMatchObject({
			ok: false,
			problem: { text: expect.stringMatching(/32 Mio/) }
		});
		await expect(api.importBackup(huge, { sections: ['git'], devices: {} })).resolves.toMatchObject(
			{ ok: false }
		);
		expect(calls).toEqual([]);
	});

	it('explains an archive the manager refuses, and a body that is too large', async () => {
		const refused = fakeFetch(() => json(400, { code: 'invalidArchive', message: 'no end' }));
		await expect(refused.api.previewBackup(zip)).resolves.toMatchObject({
			ok: false,
			code: 'invalidArchive',
			problem: { detail: 'no end' }
		});
		const tooLarge = fakeFetch(() => new Response('', { status: 413 }));
		await expect(
			tooLarge.api.importBackup(zip, { sections: ['git'], devices: {} })
		).resolves.toMatchObject({ problem: { text: expect.stringMatching(/32 Mio/) } });
	});

	it('lists, previews and restores the snapshots', async () => {
		const list = fakeFetch(() =>
			json(200, [{ id: '1-0', createdAt: 1, appVersion: '1', sections: [] }])
		);
		await expect(list.api.listSnapshots()).resolves.toMatchObject({ ok: true });
		expect(list.calls[0].input).toBe('/api/backup/snapshots');

		const look = fakeFetch(() => json(200, preview));
		await look.api.previewSnapshot('1-0');
		expect(look.calls[0].input).toBe('/api/backup/snapshots/1-0/preview');

		const back = fakeFetch(() => json(200, report));
		await expect(back.api.restoreSnapshot('1-0')).resolves.toEqual({ ok: true, value: report });
		expect(back.calls[0]).toMatchObject({
			input: '/api/backup/snapshots/1-0/restore',
			init: { method: 'POST' }
		});
	});

	it('reports a manager that cannot be reached, and an answer it cannot read', async () => {
		const down = fakeFetch(() => Promise.reject(new TypeError('Failed to fetch')));
		await expect(down.api.listSnapshots()).resolves.toMatchObject({
			problem: { text: expect.stringMatching(/injoignable/) }
		});
		const odd = fakeFetch(() => new Response('<html>', { status: 200 }));
		await expect(odd.api.listSnapshots()).resolves.toMatchObject({
			problem: { text: expect.stringMatching(/inattendue/) }
		});
	});
});
