<!--
@component
The backup of the configuration of the application: export, import (with the preview and the way
back) and the automatic snapshots taken before each import.
-->
<script lang="ts">
	import type { ManagerApi } from '#lib/api/manager-api.js';
	import type { DeviceInfo, MouseInfo } from '#lib/generated/index.js';
	import BackupExport from './BackupExport.svelte';
	import BackupImport from './BackupImport.svelte';
	import SnapshotList from './SnapshotList.svelte';

	interface Props {
		api: ManagerApi;
		/** The Stream Deck that are connected. */
		decks: readonly DeviceInfo[];
		/** The mice that are connected. */
		mice: readonly MouseInfo[];
	}

	let { api, decks, mice }: Props = $props();

	/** Connected devices and mice by serial, to name what a backup holds. */
	const names = $derived(
		new Map([
			...decks.map((deck): [string, string] => [deck.serial, deck.displayName]),
			...mice.map((mouse): [string, string] => [mouse.serial, mouse.displayName])
		])
	);

	/** Raised whenever a snapshot may have been taken: the list reads itself again. */
	let snapshotsVersion = $state(0);
	const refreshSnapshots = () => (snapshotsVersion += 1);
</script>

<div class="flex flex-col gap-4">
	<BackupExport {api} />
	<BackupImport {api} {names} {decks} onchange={refreshSnapshots} />
	<SnapshotList {api} {names} version={snapshotsVersion} onchange={refreshSnapshots} />
</div>
