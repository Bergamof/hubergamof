import type {
	ApiError,
	DeviceConfig,
	ErrorCode,
	FileChooserRequest,
	FileChosen,
	ImageUploaded
} from '#lib/generated/index.js';
import { problemOf, type Problem } from '#lib/deck/problems.js';

const MEBIBYTE = 1024 * 1024;

/** Largest key image: `MAX_SOURCE_BYTES` of the manager (constants are not exported to TS). */
export const MAX_IMAGE_BYTES = 4 * MEBIBYTE;

/** Largest archive: `MAX_ARCHIVE_BYTES` of the manager. */
export const MAX_ARCHIVE_BYTES = 32 * MEBIBYTE;

/** Formats accepted for key images, as `accept` of a file input. */
export const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/svg+xml'] as const;

/** Type sent for a file the system could not identify: the manager reads its content. */
const UNKNOWN_TYPE = 'application/octet-stream';

const HTTP_PAYLOAD_TOO_LARGE = 413;

/** A request that failed: what to tell the user, and the manager's code when it gave one. */
export interface ApiFailure {
	ok: false;
	problem: Problem;
	code?: ErrorCode;
}

export type ApiResult<T> = { ok: true; value: T } | ApiFailure;

export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

/** `bytes` in mebibytes, as shown to the user (« 4 Mio »). */
export function mebibytes(bytes: number): string {
	return `${Math.round(bytes / MEBIBYTE)} Mio`;
}

const NO_CONNECTION: Problem = {
	text: 'Le gestionnaire est injoignable : vérifiez qu’il est lancé.',
	detail: null
};
const IMAGE_TOO_LARGE: Problem = {
	text: `Image trop volumineuse : ${mebibytes(MAX_IMAGE_BYTES)} au plus.`,
	detail: null
};
const UNSUPPORTED_IMAGE: Problem = {
	text: 'Format d’image non pris en charge : PNG, JPEG ou SVG.',
	detail: null
};
const ARCHIVE_TOO_LARGE: Problem = {
	text: `Fichier trop volumineux : ${mebibytes(MAX_ARCHIVE_BYTES)} au plus.`,
	detail: null
};
const UNEXPECTED_ANSWER: Problem = {
	text: 'Le gestionnaire a fait une réponse inattendue.',
	detail: null
};

function devicePath(serial: string): string {
	return `/api/devices/${encodeURIComponent(serial)}`;
}

/** URL of image `name` of a device, rendered like a key by the manager. */
export function keyImageUrl(serial: string, name: string): string {
	return `${devicePath(serial)}/images/${encodeURIComponent(name)}`;
}

/** URL downloading the configuration archive of a device. */
export function exportUrl(serial: string): string {
	return `${devicePath(serial)}/export`;
}

/** Name of an archive of device `serial` saved on `date` (local day). */
export function exportFileName(serial: string, date: Date): string {
	const day = [date.getFullYear(), date.getMonth() + 1, date.getDate()]
		.map((part) => String(part).padStart(2, '0'))
		.join('-');
	return `hubergamof-${serial}-${day}.zip`;
}

function isApiError(value: unknown): value is ApiError {
	return (
		typeof value === 'object' &&
		value !== null &&
		typeof (value as ApiError).code === 'string' &&
		typeof (value as ApiError).message === 'string'
	);
}

async function failureFrom(response: Response, tooLarge: Problem): Promise<ApiFailure> {
	if (response.status === HTTP_PAYLOAD_TOO_LARGE) return { ok: false, problem: tooLarge };
	try {
		const body: unknown = await response.json();
		if (isApiError(body)) {
			return { ok: false, problem: problemOf(body.code, body.message), code: body.code };
		}
	} catch {
		// Not an API answer (proxy, crash…): reported below with its status.
	}
	const problem = {
		text: 'Le gestionnaire a signalé une erreur.',
		detail: `HTTP ${response.status}`
	};
	return { ok: false, problem };
}

/** Client of the internal HTTP routes of the manager (`/api/…`). */
export class ManagerApi {
	readonly #fetch: Fetch;

	constructor(fetchImpl: Fetch = (input, init) => fetch(input, init)) {
		this.#fetch = fetchImpl;
	}

	/**
	 * Stores a key image for device `serial`; resolves its name. A file the system could not
	 * identify (no type) is sent anyway: the manager reads the format from the content.
	 */
	async uploadKeyImage(serial: string, file: File): Promise<ApiResult<string>> {
		const known = file.type === '' || (IMAGE_TYPES as readonly string[]).includes(file.type);
		if (!known) return { ok: false, problem: UNSUPPORTED_IMAGE };
		if (file.size > MAX_IMAGE_BYTES) return { ok: false, problem: IMAGE_TOO_LARGE };
		const path = `${devicePath(serial)}/images`;
		const result = await this.#post<ImageUploaded>(
			path,
			file,
			file.type || UNKNOWN_TYPE,
			IMAGE_TOO_LARGE
		);
		return result.ok ? { ok: true, value: result.value.name } : result;
	}

	/**
	 * Opens the desktop's file dialog of the file setting `setting` of module `module` and
	 * resolves the path the user chose, `null` when they closed it. The failure says (`code`)
	 * when there is no dialog on this desktop (`unavailable`) or one is already open (`busy`).
	 */
	async chooseFile(module: string, setting: string): Promise<ApiResult<string | null>> {
		const request: FileChooserRequest = { module, setting };
		const result = await this.#post<FileChosen>(
			'/api/file-chooser',
			new Blob([JSON.stringify(request)]),
			'application/json',
			UNEXPECTED_ANSWER
		);
		return result.ok ? { ok: true, value: result.value.path } : result;
	}

	/**
	 * Replaces the configuration of device `serial` by an exported archive. The new
	 * configuration then reaches every client over the WebSocket.
	 */
	async importArchive(serial: string, file: File): Promise<ApiResult<void>> {
		if (file.size > MAX_ARCHIVE_BYTES) return { ok: false, problem: ARCHIVE_TOO_LARGE };
		const path = `${devicePath(serial)}/import`;
		const result = await this.#post<DeviceConfig>(path, file, 'application/zip', ARCHIVE_TOO_LARGE);
		return result.ok ? { ok: true, value: undefined } : result;
	}

	async #post<T>(
		path: string,
		body: Blob,
		contentType: string,
		tooLarge: Problem
	): Promise<ApiResult<T>> {
		let response: Response;
		try {
			response = await this.#fetch(path, {
				method: 'POST',
				headers: { 'content-type': contentType },
				body
			});
		} catch {
			return { ok: false, problem: NO_CONNECTION };
		}
		if (!response.ok) return failureFrom(response, tooLarge);
		try {
			return { ok: true, value: (await response.json()) as T };
		} catch {
			return { ok: false, problem: UNEXPECTED_ANSWER };
		}
	}
}
