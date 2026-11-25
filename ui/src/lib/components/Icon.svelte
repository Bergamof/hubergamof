<!--
@component
Small line icon, drawn inline (no icon font, no request: the content security policy forbids both).
Decorative: the text next to it carries the name.
-->
<script lang="ts" module>
	const PATHS = {
		home: 'M3 11l9-8 9 8M5 10v10h14V10',
		deck: 'M3 7a2 2 0 012-2h14a2 2 0 012 2v10a2 2 0 01-2 2H5a2 2 0 01-2-2zM8 9h.01M12 9h.01M16 9h.01M8 13h.01M12 13h.01M16 13h.01',
		mouse: 'M6 9a6 6 0 0112 0v6a6 6 0 01-12 0zM12 7v4',
		headset: 'M4 15v-3a8 8 0 0116 0v3M3 14h4v6H3zM17 14h4v6h-4z',
		terminal: 'M4 17l6-5-6-5M12 19h8',
		git: 'M6 8v8M18 10c0 4-6 3-12 6M4 6a2 2 0 104 0 2 2 0 10-4 0M4 18a2 2 0 104 0 2 2 0 10-4 0M16 8a2 2 0 104 0 2 2 0 10-4 0',
		server:
			'M3 6a2 2 0 012-2h14a2 2 0 012 2v3a2 2 0 01-2 2H5a2 2 0 01-2-2zM3 15a2 2 0 012-2h14a2 2 0 012 2v3a2 2 0 01-2 2H5a2 2 0 01-2-2zM7 7.5h.01M7 16.5h.01',
		modules: 'M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM17 14v6M14 17h6',
		radial:
			'M3 12a9 9 0 1018 0 9 9 0 10-18 0M9 12a3 3 0 106 0 3 3 0 10-6 0M12 3v6M12 15v6M3 12h6M15 12h6',
		refresh: 'M20 12a8 8 0 10-3 6.2M20 5v5h-5',
		download: 'M12 4v11M7 10l5 5 5-5M4 19h16',
		lock: 'M5 11h14v9H5zM8 11V8a4 4 0 018 0v3',
		wifiOff: 'M2 9a15 15 0 0120 0M5 12.5a10 10 0 0114 0M8.5 16a5 5 0 017 0M12 19.5h.01',
		info: 'M3 12a9 9 0 1018 0 9 9 0 10-18 0M12 11v5M12 8h.01',
		check: 'M5 12.5l4.5 4.5L19 7',
		chevronRight: 'M9 6l6 6-6 6',
		battery: 'M2 8h17v9H2zM22 11v3M6 11v3',
		timer: 'M4 13a8 8 0 1016 0 8 8 0 10-16 0M12 9v4l3 2M9 2h6',
		volume: 'M4 9v6h4l5 4V5L8 9H4zM17 9a4 4 0 010 6',
		chat: 'M4 5h16v11H9l-5 4z',
		code: 'M8 8l-5 4 5 4M16 8l5 4-5 4M14 5l-4 14',
		spark: 'M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z',
		device: 'M3 7a2 2 0 012-2h14a2 2 0 012 2v10a2 2 0 01-2 2H5a2 2 0 01-2-2zM3 15h18',
		external: 'M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 01-1 1H5a1 1 0 01-1-1V7a1 1 0 011-1h5'
	} as const;

	export type IconName = keyof typeof PATHS;

	export const iconPath = (name: IconName): string => PATHS[name];
</script>

<script lang="ts">
	interface Props {
		name: IconName;
		class?: string;
	}

	let { name, class: className = 'size-4' }: Props = $props();
</script>

<svg
	viewBox="0 0 24 24"
	fill="none"
	stroke="currentColor"
	stroke-width="1.75"
	stroke-linecap="round"
	stroke-linejoin="round"
	aria-hidden="true"
	class={['shrink-0', className]}
>
	<path d={iconPath(name)} />
</svg>
