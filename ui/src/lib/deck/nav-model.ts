import { resolve } from '$app/paths';
import type { ResolvedPathname } from '$app/types';
import type { DeviceInfo, HeadsetInfo, MouseInfo } from '#lib/generated/index.js';
import type { BatteryReading } from './battery.js';

/** The pages of the interface, by address. */
export const PATHS = {
	home: '/',
	devices: '/peripheriques',
	deck: '/peripheriques/stream-deck',
	mouse: '/peripheriques/souris',
	headset: '/peripheriques/casque',
	aliases: '/poste/alias',
	git: '/poste/git',
	ssh: '/poste/ssh',
	modules: '/application/modules',
	radial: '/application/menu-circulaire',
	updates: '/application/mises-a-jour',
	backup: '/application/sauvegarde'
} as const;

/** The groups of the navigation, in the order they are shown. */
export type NavGroup = 'Périphériques' | 'Poste' | 'Application';

interface PageInfo {
	path: string;
	title: string;
	group: NavGroup | undefined;
}

/** Every page: its title (breadcrumb, document title) and the group it belongs to. */
export const PAGES: readonly PageInfo[] = [
	{ path: PATHS.home, title: 'Accueil', group: undefined },
	{ path: PATHS.devices, title: 'Périphériques', group: 'Périphériques' },
	{ path: PATHS.deck, title: 'Stream Deck', group: 'Périphériques' },
	{ path: PATHS.mouse, title: 'Souris', group: 'Périphériques' },
	{ path: PATHS.headset, title: 'Casque', group: 'Périphériques' },
	{ path: PATHS.aliases, title: 'Alias du shell', group: 'Poste' },
	{ path: PATHS.git, title: 'Git', group: 'Poste' },
	{ path: PATHS.ssh, title: 'SSH', group: 'Poste' },
	{ path: PATHS.modules, title: 'Modules', group: 'Application' },
	{ path: PATHS.radial, title: 'Menu circulaire', group: 'Application' },
	{ path: PATHS.updates, title: 'Mises à jour', group: 'Application' },
	{ path: PATHS.backup, title: 'Sauvegarde', group: 'Application' }
];

/** The page of an address (a trailing slash is ignored); the home page for an unknown one. */
export function pageOf(pathname: string): PageInfo {
	const path = pathname.length > 1 ? pathname.replace(/\/+$/, '') : pathname;
	return PAGES.find((page) => page.path === path) ?? PAGES[0];
}

/** A connected device, as the navigation lists it. */
export interface NavDevice {
	kind: 'deck' | 'mouse' | 'headset';
	/** Serial of the Stream Deck or the mouse, address of the headphones. */
	id: string;
	label: string;
	/** Its page. */
	path: DevicePath;
	/** What tells it from the other devices of its kind: `?appareil=`, empty for the headphones. */
	query: string;
	/** Its battery, once read; a Stream Deck has none. */
	battery?: BatteryReading;
}

export type DevicePath = typeof PATHS.deck | typeof PATHS.mouse | typeof PATHS.headset;

const DEVICE_PATH = { deck: PATHS.deck, mouse: PATHS.mouse, headset: PATHS.headset } as const;

/** The query of the page of a device: the Stream Deck and the mice are told apart by `?appareil=`. */
export function deviceQuery(kind: NavDevice['kind'], id: string): string {
	return kind === 'headset' ? '' : `?appareil=${encodeURIComponent(id)}`;
}

/** The address of the page of a device, with the base path of the application. */
export function deviceHref(device: NavDevice): ResolvedPathname {
	return resolve(
		device.kind === 'headset'
			? device.path
			: (`${device.path}${device.query}` as `${DevicePath}?${string}`)
	);
}

/** The address of the page of a module: the modules are told apart by `?module=`. */
export function moduleHref(kind: string): ResolvedPathname {
	return resolve(`${PATHS.modules}?module=${encodeURIComponent(kind)}`);
}

function entry(
	kind: NavDevice['kind'],
	id: string,
	label: string,
	battery?: BatteryReading
): NavDevice {
	return { kind, id, label, path: DEVICE_PATH[kind], query: deviceQuery(kind, id), battery };
}

/** The connected devices, in the order of the navigation: Stream Deck, mice, headphones. */
export function navDevices(
	decks: readonly DeviceInfo[],
	mice: readonly MouseInfo[],
	headsets: readonly HeadsetInfo[],
	headsetBattery: (address: string) => BatteryReading | undefined = () => undefined
): NavDevice[] {
	return [
		...decks.map((d) => entry('deck', d.serial, d.displayName)),
		...mice.map((m) => entry('mouse', m.serial, m.displayName, m.battery ?? undefined)),
		...headsets.map((h) => entry('headset', h.address, h.name, headsetBattery(h.address)))
	];
}
