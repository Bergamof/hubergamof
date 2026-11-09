import type { JsonValue, SettingField, SettingKind } from '#lib/generated/index.js';

/**
 * A file of a file list while the user edits it: the raw texts of its fields (a path being
 * typed may be temporarily invalid, an empty name stands for « no name »).
 */
export interface FileEntryDraft {
	path: string;
	name: string;
	/** Name of a stored image, `null` for the default icon. */
	icon: string | null;
}

/**
 * What a form field holds while the user edits it: a checkbox state, the raw text of the
 * input (an integer being typed may be temporarily invalid, a path list is one path per line),
 * or the files of a file list.
 */
export type DraftValue = boolean | string | FileEntryDraft[];

/** Longest name of a file (`MAX_ENTRY_NAME_CHARS` of the module API). */
export const MAX_ENTRY_NAME_CHARS = 40;

/** Most files in a list (`MAX_FILE_ENTRIES` of the module API). */
export const MAX_FILE_ENTRIES = 50;

/** Form values, by setting key. */
export type SettingsDraft = Record<string, DraftValue>;

/** Error messages of the invalid fields, by setting key. */
export type SettingsErrors = Record<string, string>;

export type ParsedSettings =
	{ ok: true; settings: Record<string, JsonValue> } | { ok: false; errors: SettingsErrors };

const INTEGER_PATTERN = /^[+-]?\d+$/;

function isRecord(value: JsonValue): value is Record<string, JsonValue> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** The stored value of a setting, or its default when missing or of the wrong type. */
function storedValue(kind: SettingKind, stored: JsonValue | undefined): JsonValue {
	switch (kind.type) {
		case 'boolean':
			return typeof stored === 'boolean' ? stored : kind.default;
		case 'integer':
			return typeof stored === 'number' ? stored : kind.default;
		case 'pathList':
			return Array.isArray(stored) && stored.every((path) => typeof path === 'string')
				? stored
				: kind.default;
		case 'fileEntryList':
			return Array.isArray(stored) && stored.every(isStoredFile) ? stored : kind.default;
		case 'text':
		case 'path':
		case 'choice':
			return typeof stored === 'string' ? stored : kind.default;
	}
}

/** A file as stored: a path, or (since names and icons) an object with a path. */
function isStoredFile(value: JsonValue): boolean {
	return typeof value === 'string' || (isRecord(value) && typeof value.path === 'string');
}

function toFileEntryDraft(stored: JsonValue): FileEntryDraft {
	if (!isRecord(stored)) return { path: String(stored), name: '', icon: null };
	return {
		path: String(stored.path),
		name: typeof stored.name === 'string' ? stored.name : '',
		icon: typeof stored.icon === 'string' ? stored.icon : null
	};
}

function toDraftValue(kind: SettingKind, value: JsonValue): DraftValue {
	switch (kind.type) {
		case 'boolean':
			return value === true;
		case 'pathList':
			return Array.isArray(value) ? value.join('\n') : '';
		case 'fileEntryList':
			return Array.isArray(value) ? value.map(toFileEntryDraft) : [];
		default:
			return String(value);
	}
}

/** Form values for `settings` (missing settings take their default). */
export function toDraft(fields: readonly SettingField[], settings: JsonValue): SettingsDraft {
	const values = isRecord(settings) ? settings : {};
	return Object.fromEntries(
		fields.map((field) => [
			field.key,
			toDraftValue(field.kind, storedValue(field.kind, values[field.key]))
		])
	);
}

type FieldResult = { ok: true; value: JsonValue } | { ok: false; error: string };

function parseInteger(kind: Extract<SettingKind, { type: 'integer' }>, text: string): FieldResult {
	const trimmed = text.trim();
	if (!INTEGER_PATTERN.test(trimmed)) return { ok: false, error: 'Saisir un nombre entier.' };
	const value = Number(trimmed);
	if (!Number.isSafeInteger(value)) return { ok: false, error: 'Nombre trop grand.' };
	if (kind.min !== null && value < kind.min) {
		return { ok: false, error: `Valeur minimale : ${kind.min}.` };
	}
	if (kind.max !== null && value > kind.max) {
		return { ok: false, error: `Valeur maximale : ${kind.max}.` };
	}
	return { ok: true, value };
}

function parseField(kind: SettingKind, draft: DraftValue): FieldResult {
	switch (kind.type) {
		case 'boolean':
			return { ok: true, value: draft === true };
		case 'integer':
			return parseInteger(kind, String(draft));
		case 'choice': {
			const value = String(draft);
			return kind.options.some((option) => option.value === value)
				? { ok: true, value }
				: { ok: false, error: 'Choisir une des valeurs proposées.' };
		}
		case 'pathList': {
			const paths = String(draft)
				.split('\n')
				.map((path) => path.trim())
				.filter((path) => path !== '');
			const invalid = paths.find((path) => !isResolvablePath(path));
			return invalid === undefined
				? { ok: true, value: paths }
				: { ok: false, error: pathError(invalid) };
		}
		case 'fileEntryList':
			return parseFileEntries(Array.isArray(draft) ? draft : []);
		case 'text':
			return { ok: true, value: String(draft) };
		case 'path': {
			const path = String(draft).trim();
			return path === '' || isResolvablePath(path)
				? { ok: true, value: path }
				: { ok: false, error: pathError(path) };
		}
	}
}

/** Why `entry` cannot be saved, `undefined` when it can (same rules as the module API). */
export function fileEntryError(entry: FileEntryDraft): string | undefined {
	const path = entry.path.trim();
	if (path === '') return 'Indiquer le chemin du fichier.';
	if (!isResolvablePath(path)) return pathError(path);
	if ([...entry.name.trim()].length > MAX_ENTRY_NAME_CHARS) {
		return `Nom trop long : ${MAX_ENTRY_NAME_CHARS} caractères au plus.`;
	}
	return undefined;
}

function parseFileEntries(entries: readonly FileEntryDraft[]): FieldResult {
	if (entries.length > MAX_FILE_ENTRIES) {
		return { ok: false, error: `${MAX_FILE_ENTRIES} fichiers au plus.` };
	}
	const files: JsonValue[] = [];
	const seen = new Set<string>();
	for (const entry of entries) {
		const error = fileEntryError(entry);
		if (error) return { ok: false, error };
		const path = entry.path.trim();
		if (seen.has(path)) return { ok: false, error: `« ${path} » est déjà dans la liste.` };
		seen.add(path);
		files.push({ path, name: entry.name.trim() || null, icon: entry.icon });
	}
	return { ok: true, value: files };
}

/** Whether two form values are the same (files are compared by content). */
export function sameDraftValue(a: DraftValue | undefined, b: DraftValue | undefined): boolean {
	if (Array.isArray(a) && Array.isArray(b)) {
		return (
			a.length === b.length &&
			a.every(
				(entry, index) =>
					entry.path === b[index]?.path &&
					entry.name === b[index]?.name &&
					entry.icon === b[index]?.icon
			)
		);
	}
	return a === b;
}

/**
 * Whether the manager can resolve `path` without a working directory: absolute, or in the
 * home directory (same rule as `is_setting_path` in the module API).
 */
function isResolvablePath(path: string): boolean {
	if (path.startsWith('/')) return true;
	if (!path.startsWith('~/')) return false;
	// Like Rust's `Path::components`: empty and `.` segments are ignored, so `~/.` is `~`.
	return path
		.slice(2)
		.split('/')
		.some((segment) => segment !== '' && segment !== '.');
}

function pathError(path: string): string {
	return `« ${path} » : chemin absolu attendu, commençant par / ou ~/.`;
}

/** Help shown under a field: its own help, then the constraints of its kind. */
export function settingHint(field: SettingField): string {
	const kind = field.kind;
	const constraints: string[] = [];
	if (kind.type === 'integer') {
		if (kind.min !== null && kind.max !== null)
			constraints.push(`Entre ${kind.min} et ${kind.max}.`);
		else if (kind.min !== null) constraints.push(`Au moins ${kind.min}.`);
		else if (kind.max !== null) constraints.push(`Au plus ${kind.max}.`);
	} else if (kind.type === 'pathList') {
		constraints.push('Un chemin absolu par ligne (ou commençant par ~/).');
	} else if (kind.type === 'fileEntryList') {
		constraints.push(`${MAX_FILE_ENTRIES} fichiers au plus.`);
	} else if (kind.type === 'path') {
		constraints.push(
			kind.directory
				? 'Chemin absolu d’un dossier (ou commençant par ~/).'
				: 'Chemin absolu d’un fichier (ou commençant par ~/).'
		);
	}
	return [field.help, ...constraints].filter(Boolean).join(' ');
}

/**
 * Checks the form values like the manager does (types, bounds, choices) and builds the
 * settings to save. Settings unknown to the form (written by another version) are kept.
 */
export function parseDraft(
	fields: readonly SettingField[],
	draft: SettingsDraft,
	original: JsonValue
): ParsedSettings {
	const settings: Record<string, JsonValue> = isRecord(original) ? { ...original } : {};
	const errors: SettingsErrors = {};
	for (const field of fields) {
		const result = parseField(
			field.kind,
			draft[field.key] ?? toDraftValue(field.kind, field.kind.default)
		);
		if (result.ok) settings[field.key] = result.value;
		else errors[field.key] = result.error;
	}
	return Object.keys(errors).length === 0 ? { ok: true, settings } : { ok: false, errors };
}

/** The label to save: trimmed, `null` when empty (the module default is shown). */
export function parseLabel(text: string): string | null {
	const label = text.trim();
	return label === '' ? null : label;
}
