import { describe, expect, it } from 'vitest';
import type { SettingField, SettingKind } from '#lib/generated/index.js';
import { parseDraft, parseLabel, settingHint, toDraft } from './settings-form.js';

function field(key: string, kind: SettingKind): SettingField {
	return { key, label: key, help: null, kind };
}

const fields: SettingField[] = [
	field('enabled', { type: 'boolean', default: true }),
	field('minutes', { type: 'integer', default: 25, min: 1, max: 60, unit: 'min' }),
	field('name', { type: 'text', default: '' }),
	field('folder', { type: 'path', default: '~', directory: true }),
	field('files', { type: 'pathList', default: [] }),
	field('target', {
		type: 'choice',
		default: 'sink',
		options: [
			{ value: 'sink', label: 'Sortie' },
			{ value: 'source', label: 'Entrée' }
		]
	})
];

describe('toDraft', () => {
	it('shows stored values, and defaults for missing or mistyped ones', () => {
		expect(toDraft(fields, { minutes: 50, files: ['a.yaml', 'b.yaml'], target: 3 })).toEqual({
			enabled: true,
			minutes: '50',
			name: '',
			folder: '~',
			files: 'a.yaml\nb.yaml',
			target: 'sink'
		});
		expect(toDraft(fields, null).minutes).toBe('25');
	});
});

describe('parseDraft', () => {
	const valid = {
		enabled: false,
		minutes: ' 30 ',
		name: 'Travail',
		folder: ' ~/projets ',
		files: '/a.yaml\n\n  ~/b.yaml \n',
		target: 'source'
	};

	it('builds typed settings and keeps the settings the form does not know', () => {
		expect(parseDraft(fields, valid, { future: 'kept', minutes: 25 })).toEqual({
			ok: true,
			settings: {
				future: 'kept',
				enabled: false,
				minutes: 30,
				name: 'Travail',
				folder: '~/projets',
				files: ['/a.yaml', '~/b.yaml'],
				target: 'source'
			}
		});
	});

	it.each([
		['douze', 'Saisir un nombre entier.'],
		['2.5', 'Saisir un nombre entier.'],
		['', 'Saisir un nombre entier.'],
		['0', 'Valeur minimale : 1.'],
		['61', 'Valeur maximale : 60.'],
		['99999999999999999999', 'Nombre trop grand.']
	])('refuses the integer %j', (minutes, error) => {
		expect(parseDraft(fields, { ...valid, minutes }, {})).toEqual({
			ok: false,
			errors: { minutes: error }
		});
	});

	it.each([
		['files', 'a.yaml', '« a.yaml » : chemin absolu attendu, commençant par / ou ~/.'],
		['files', '/ok.yaml\n~user/x', '« ~user/x » : chemin absolu attendu, commençant par / ou ~/.'],
		['folder', 'projets', '« projets » : chemin absolu attendu, commençant par / ou ~/.'],
		['folder', '~', '« ~ » : chemin absolu attendu, commençant par / ou ~/.'],
		['files', '~/', '« ~/ » : chemin absolu attendu, commençant par / ou ~/.'],
		['files', '~/./', '« ~/./ » : chemin absolu attendu, commençant par / ou ~/.']
	])('refuses the relative path in %s: %j', (key, value, error) => {
		expect(parseDraft(fields, { ...valid, [key]: value }, {})).toEqual({
			ok: false,
			errors: { [key]: error }
		});
	});

	it('accepts home paths and an empty single path', () => {
		const parsed = parseDraft(fields, { ...valid, folder: '', files: '~/a\n/b' }, {});
		expect(parsed).toMatchObject({ ok: true, settings: { folder: '', files: ['~/a', '/b'] } });
	});

	it('refuses a value outside the choices', () => {
		expect(parseDraft(fields, { ...valid, target: 'other' }, {})).toEqual({
			ok: false,
			errors: { target: 'Choisir une des valeurs proposées.' }
		});
	});

	it('accepts unbounded integers and fills fields missing from the draft', () => {
		const unbounded = [
			field('count', { type: 'integer', default: 3, min: null, max: null, unit: null })
		];
		expect(parseDraft(unbounded, { count: '-40' }, {})).toEqual({
			ok: true,
			settings: { count: -40 }
		});
		expect(parseDraft(unbounded, {}, {})).toEqual({ ok: true, settings: { count: 3 } });
	});
});

describe('parseLabel', () => {
	it('trims the label and turns an empty one into the module default', () => {
		expect(parseLabel('  Focus ')).toBe('Focus');
		expect(parseLabel('   ')).toBeNull();
	});
});

describe('settingHint', () => {
	it('joins the help and the constraints of the kind', () => {
		const help = (kind: SettingKind, text: string | null = null) =>
			settingHint({ key: 'k', label: 'k', help: text, kind });
		const integer = (min: number | null, max: number | null): SettingKind => ({
			type: 'integer',
			default: 1,
			min,
			max,
			unit: null
		});
		expect(help(integer(1, 9), 'Durée.')).toBe('Durée. Entre 1 et 9.');
		expect(help(integer(1, null))).toBe('Au moins 1.');
		expect(help(integer(null, 9))).toBe('Au plus 9.');
		expect(help(integer(null, null))).toBe('');
		expect(help({ type: 'path', default: '', directory: true })).toMatch(/dossier/);
		expect(help({ type: 'path', default: '', directory: false })).toMatch(/fichier/);
		expect(help({ type: 'pathList', default: [] })).toMatch(/par ligne/);
		expect(help({ type: 'boolean', default: true }, 'Aide.')).toBe('Aide.');
	});
});
