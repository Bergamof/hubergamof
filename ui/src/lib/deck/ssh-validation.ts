import type { AdvancedSetting, SshHost, SshSystemHost } from '#lib/generated/index.js';
import { lengthOf } from './alias-validation.js';
import {
	ADVANCED_KEY_MAX,
	ADVANCED_MAX,
	hasPairedQuotes,
	isOneLineValue,
	validateSetting,
	valueError,
	type SettingRules
} from './advanced-validation.js';

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

const MAX_IPV6_CHARS = 45;

/** An IPv6 address between brackets: hexadecimal digits, `:` and `.`, with at least one `:`. */
function isIpv6Address(address: string): boolean {
	return (
		address.includes(':') && lengthOf(address) <= MAX_IPV6_CHARS && /^[0-9A-Fa-f:.]+$/.test(address)
	);
}

/**
 * The port of a hop once its user is taken off, or `undefined` when the rest is not `host`,
 * `host:port`, `[ipv6]` or `[ipv6]:port`. `ssh` refuses a whole configuration for a jump it cannot
 * read, so an address with `:` is accepted only between brackets. `null`: no port.
 */
function portOfHop(rest: string): string | null | undefined {
	if (rest.startsWith('[')) {
		const close = rest.indexOf(']');
		if (close < 0 || !isIpv6Address(rest.slice(1, close))) return undefined;
		const after = rest.slice(close + 1);
		if (after === '') return null;
		return after.startsWith(':') ? after.slice(1) : undefined;
	}
	const colon = rest.indexOf(':');
	const host = colon < 0 ? rest : rest.slice(0, colon);
	if (!isValidHostName(host) || host.includes(':')) return undefined;
	return colon < 0 ? null : rest.slice(colon + 1);
}

/** One hop: `[user@]host[:port]`. */
function isValidJump(entry: string): boolean {
	const at = entry.indexOf('@');
	const rest = at < 0 ? entry : entry.slice(at + 1);
	if (at >= 0 && !isValidUser(entry.slice(0, at))) return false;
	const port = portOfHop(rest);
	return port !== undefined && (port === null || isValidPortText(port));
}

/** `ProxyJump`: hops joined by `,`. */
export function isValidProxyJump(jumps: string): boolean {
	if (jumps === '' || lengthOf(jumps) > SSH_VALUE_MAX) return false;
	const hops = jumps.split(',');
	return hops.length <= SSH_JUMPS_MAX && hops.every(isValidJump);
}

/** What is wrong with a list of jumps, in French; `undefined` when it is fine. */
function proxyJumpError(jumps: string): string | undefined {
	if (isValidProxyJump(jumps)) return undefined;
	if (jumps.split(',').length > SSH_JUMPS_MAX) return `${SSH_JUMPS_MAX} rebonds au plus.`;
	const wrong = jumps.split(',').find((hop) => !isValidJump(hop));
	const which = wrong === undefined || wrong === '' ? '' : `« ${wrong} » n’est pas valide : `;
	return `${which}chaque rebond s’écrit [utilisateur@]machine[:port], séparés par « , » ; une adresse IPv6 s’écrit entre crochets, [::1]:22 ; ni espace ni « - » au début.`;
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
	/** Kept as they are: the list of the options has its own form. */
	options: AdvancedSetting[];
}

/** A host as the form starts from it; a host found elsewhere has no options of its own yet. */
export function draftOf(
	host: Omit<SshHost, 'options'> & Partial<Pick<SshHost, 'options'>>
): HostDraft {
	return {
		...host,
		options: host.options ?? [],
		port: host.port === null ? '' : String(host.port)
	};
}

export const BLANK_HOST: HostDraft = {
	alias: '',
	hostName: '',
	user: '',
	port: '',
	identityFile: '',
	proxyJump: '',
	forwardAgent: false,
	enabled: true,
	options: []
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
			'Un nom de machine ou une adresse (IPv4, IPv6) de 255 caractères au plus : lettres, chiffres, « . », « _ », « - » et « : », sans espace, et ne commence pas par « - ».';
	if (draft.user !== '' && !isValidUser(draft.user))
		found.user = `${SSH_NAME_MAX} caractères au plus : lettres, chiffres, « . », « _ » et « - », sans espace, et ne commence pas par « - ».`;
	if (draft.port !== '' && !isValidPortText(draft.port))
		found.port = 'Le port est un nombre entier de 1 à 65535.';
	if (draft.identityFile !== '' && !isValidIdentityFile(draft.identityFile))
		found.identityFile = `Le chemin de la clé commence par ${SSH_FOLDER} (par exemple ${SSH_FOLDER}id_ed25519) ; chaque nom : lettres, chiffres, « . », « _ » et « - », sans « .. » ni espace.`;
	if (draft.proxyJump !== '') {
		const jumpError = proxyJumpError(draft.proxyJump);
		if (jumpError) found.proxyJump = jumpError;
	}
	return found;
}

export function hasHostErrors(errors: HostErrors): boolean {
	return Object.keys(errors).length > 0;
}

/** The name `ssh` documents for an option the manager could not carry, from the lowercase one it gives. */
const OPTION_REASONS: Record<string, string> = {
	identityfile: 'plusieurs clés',
	forwardagent: 'une valeur autre que yes ou no',
	port: 'une valeur qui n’est pas un nombre'
};

/** What is said of an option that cannot be taken: its name, and why when it is a known case. */
function lostOption(option: string): string {
	const reason = OPTION_REASONS[option] ?? 'écrite deux fois, ou hors des règles de HuBergamof';
	return `${option} (${reason})`;
}

/** What is wrong with the first option of a list, in French; `undefined` when the list is fine. */
function optionListError(options: readonly AdvancedSetting[]): string | undefined {
	const taken: string[] = [];
	if (options.length > ADVANCED_MAX) return `${ADVANCED_MAX} options au plus.`;
	for (const option of options) {
		const problem = Object.values(
			validateSetting({ key: option.key, value: option.value }, taken, SSH_OPTION_RULES)
		)[0];
		if (problem) return `L’option « ${option.key} » : ${problem}`;
		taken.push(option.key);
	}
	return undefined;
}

/** Why a host found in `~/.ssh/config` cannot be taken over, if it cannot. */
export function takeOverObstacle(
	host: SshSystemHost,
	takenAliases: readonly string[]
): string | undefined {
	if (host.lostOptions.length > 0) {
		const names = host.lostOptions.map(lostOption).join(' ; ');
		return `Ne peut pas être repris sans perte : ${names}. Corrigez-le dans ~/.ssh/config.`;
	}
	const draft = draftOf({ ...host, enabled: true });
	const [problem] = Object.values(validateHost(draft, takenAliases));
	const optionsProblem = optionListError(host.options);
	const found = problem ?? optionsProblem;
	return found ? `Ne respecte pas les règles de HuBergamof : ${found}` : undefined;
}

/** The options the form of a host manages (`MANAGED_OPTIONS` in `ssh.rs`), in lower case. */
const MANAGED_OPTIONS: readonly string[] = [
	'hostname',
	'user',
	'port',
	'identityfile',
	'proxyjump',
	'forwardagent'
];

const OPTION_KEY = /^[A-Za-z][A-Za-z0-9]*$/;

/** Mirrors `is_valid_ssh_option_key`: a keyword of `ssh_config`, neither `Host`, `Match` nor a managed one. */
export function isValidOptionKey(key: string): boolean {
	const lower = key.toLowerCase();
	return (
		OPTION_KEY.test(key) &&
		lengthOf(key) <= ADVANCED_KEY_MAX &&
		lower !== 'host' &&
		lower !== 'match' &&
		!MANAGED_OPTIONS.includes(lower)
	);
}

/** Mirrors `is_valid_ssh_option_value`: one line, quotes in pairs. */
export function isValidOptionValue(value: string): boolean {
	return isOneLineValue(value) && hasPairedQuotes(value);
}

function optionKeyError(key: string): string | undefined {
	if (key === '') return 'Saisissez le nom de l’option.';
	const lower = key.toLowerCase();
	if (lower === 'host' || lower === 'match') {
		return '« Host » et « Match » ouvrent une autre section : ils ne sont pas acceptés ici.';
	}
	if (MANAGED_OPTIONS.includes(lower)) {
		return 'Cette option a son propre champ dans le formulaire du serveur.';
	}
	if (!isValidOptionKey(key)) {
		return `Une option commence par une lettre, puis lettres et chiffres (${ADVANCED_KEY_MAX} au plus) : ServerAliveInterval.`;
	}
	return undefined;
}

/** What the options written by the person must follow: those of `ssh_config`, compared without regard to case. */
export const SSH_OPTION_RULES: SettingRules = {
	keyError: optionKeyError,
	valueError: (value) => valueError(value, true),
	fold: (key) => key.toLowerCase()
};
