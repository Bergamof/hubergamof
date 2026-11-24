import type { SshHost, SshSystemHost } from '#lib/generated/index.js';
import { lengthOf } from './alias-validation.js';

/** Limits of the manager (`MAX_SSH_*`, `SSH_FOLDER` in `ssh.rs`). */
export const SSH_HOSTS_MAX = 128;
export const SSH_NAME_MAX = 64;
export const SSH_VALUE_MAX = 255;
export const SSH_JUMPS_MAX = 5;
export const SSH_FOLDER = '~/.ssh/';
/** What a field of text accepts to be typed: twice what is valid, so that the check says why. */
export const SSH_INPUT_MAX = SSH_VALUE_MAX * 2;

const WORD = /^[A-Za-z0-9._-]+$/;
const HOST_NAME = /^[A-Za-z0-9._:-]+$/;
const PORT = /^\+?[0-9]+$/;

function isWord(text: string, max: number): boolean {
	return text !== '' && lengthOf(text) <= max && WORD.test(text) && !text.startsWith('-');
}

/** The name of a host: a word, so no joker and no space. */
export function isValidAlias(alias: string): boolean {
	return isWord(alias, SSH_NAME_MAX);
}

export function isValidUser(user: string): boolean {
	return isWord(user, SSH_NAME_MAX);
}

/** `HostName`: a name or an address, IPv6 included. */
export function isValidHostName(host: string): boolean {
	return (
		host !== '' && lengthOf(host) <= SSH_VALUE_MAX && HOST_NAME.test(host) && !host.startsWith('-')
	);
}

/** `IdentityFile`: `~/.ssh/` and names (no `.` nor `..`) joined by `/`. */
export function isValidIdentityFile(path: string): boolean {
	if (!path.startsWith(SSH_FOLDER)) return false;
	const relative = path.slice(SSH_FOLDER.length);
	return (
		lengthOf(path) <= SSH_VALUE_MAX &&
		relative !== '' &&
		relative.split('/').every((part) => isWord(part, SSH_NAME_MAX) && part !== '.' && part !== '..')
	);
}

/** A port as typed: an integer from 1 to 65535. */
function isValidPortText(text: string): boolean {
	return PORT.test(text) && Number(text) >= 1 && Number(text) <= 65_535;
}

/** One hop: `[user@]host[:port]`. */
function isValidJump(entry: string): boolean {
	const at = entry.indexOf('@');
	const user = at < 0 ? '' : entry.slice(0, at);
	const rest = at < 0 ? entry : entry.slice(at + 1);
	const colon = rest.lastIndexOf(':');
	const hasPort = colon >= 0 && !rest.slice(0, colon).includes(':');
	const host = hasPort ? rest.slice(0, colon) : rest;
	const port = hasPort ? rest.slice(colon + 1) : undefined;
	return (
		(at < 0 || isValidUser(user)) &&
		isValidHostName(host) &&
		(port === undefined || isValidPortText(port))
	);
}

/** `ProxyJump`: hops joined by `,`. */
export function isValidProxyJump(jumps: string): boolean {
	if (jumps === '' || lengthOf(jumps) > SSH_VALUE_MAX) return false;
	const hops = jumps.split(',');
	return hops.length <= SSH_JUMPS_MAX && hops.every(isValidJump);
}

/** What is wrong with the fields of a host, in French; a field without a problem is absent. */
export interface HostErrors {
	alias?: string;
	hostName?: string;
	user?: string;
	port?: string;
	identityFile?: string;
	proxyJump?: string;
}

/** A host as typed: the port is text until it is checked. */
export interface HostDraft {
	alias: string;
	hostName: string;
	user: string;
	port: string;
	identityFile: string;
	proxyJump: string;
	forwardAgent: boolean;
	enabled: boolean;
}

export function draftOf(host: SshHost): HostDraft {
	return { ...host, port: host.port === null ? '' : String(host.port) };
}

export const BLANK_HOST: HostDraft = {
	alias: '',
	hostName: '',
	user: '',
	port: '',
	identityFile: '',
	proxyJump: '',
	forwardAgent: false,
	enabled: true
};

/** The host a draft stands for, once it has no error (see {@link validateHost}). */
export function hostOf(draft: HostDraft): SshHost {
	return { ...draft, port: draft.port === '' ? null : Number(draft.port) };
}

export function validateHost(draft: HostDraft, takenAliases: readonly string[]): HostErrors {
	const found: HostErrors = {};
	if (draft.alias === '') found.alias = 'Saisissez le nom du serveur.';
	else if (!isValidAlias(draft.alias))
		found.alias = `Le nom tient en ${SSH_NAME_MAX} caractères au plus : lettres, chiffres, « . », « _ » et « - », sans espace ni joker, et ne commence pas par « - ».`;
	else if (takenAliases.includes(draft.alias)) found.alias = 'Un serveur porte déjà ce nom.';
	if (draft.hostName !== '' && !isValidHostName(draft.hostName))
		found.hostName =
			'Un nom de machine ou une adresse : lettres, chiffres, « . », « _ », « - » et « : », sans espace.';
	if (draft.user !== '' && !isValidUser(draft.user))
		found.user =
			'Lettres, chiffres, « . », « _ » et « - », sans espace, et ne commence pas par « - ».';
	if (draft.port !== '' && !isValidPortText(draft.port))
		found.port = 'Le port est un nombre entier de 1 à 65535.';
	if (draft.identityFile !== '' && !isValidIdentityFile(draft.identityFile))
		found.identityFile = `Le chemin de la clé commence par ${SSH_FOLDER} (par exemple ${SSH_FOLDER}id_ed25519), sans « .. », sans espace.`;
	if (draft.proxyJump !== '' && !isValidProxyJump(draft.proxyJump))
		found.proxyJump = `Des machines séparées par « , » (${SSH_JUMPS_MAX} au plus), chacune écrite [utilisateur@]machine[:port], sans espace ni « - » au début.`;
	return found;
}

export function hasHostErrors(errors: HostErrors): boolean {
	return Object.keys(errors).length > 0;
}

/** Why a host found in `~/.ssh/config` cannot be taken over, if it cannot. */
export function takeOverObstacle(
	host: SshSystemHost,
	takenAliases: readonly string[]
): string | undefined {
	if (host.otherOptions.length > 0) {
		return `Utilise des options que HuBergamof ne gère pas (${host.otherOptions.join(', ')}) : les commenter les ferait perdre.`;
	}
	const draft = draftOf({ ...host, enabled: true });
	return hasHostErrors(validateHost(draft, takenAliases))
		? 'Une valeur ne respecte pas les règles de HuBergamof (nom, adresse, clé hors de ~/.ssh…).'
		: undefined;
}
