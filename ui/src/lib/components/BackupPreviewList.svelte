<!--
@component
What an import (or a restore) would do, section by section, before anything is written: whether
each section is added, replaced or the same, whether its device is connected, and, set apart,
everything it would have this computer run or reach (aliases, SSH hosts and options, Git settings),
enabled or not. With `selected`, sections are ticked one by one, and the configuration of a device
that is away may be given to a connected device instead of being kept for its return.
-->
<script lang="ts">
	import type { BackupPreview, DeviceInfo } from '#lib/generated/index.js';
	import {
		CHANGE_LABELS,
		EXECUTABLE_LABELS,
		deviceSerialOf,
		sectionLabel
	} from '#lib/deck/backup-labels.js';

	interface Props {
		preview: BackupPreview;
		/** Name of each connected device and mouse, by serial. */
		names: ReadonlyMap<string, string>;
		/** Ids of the sections to apply; leave out when every section is applied (a restore). */
		selected?: string[];
		/** The Stream Deck that are connected: they may receive the configuration of one that is away. */
		decks?: readonly DeviceInfo[];
		/** Serial of a device of the backup → serial of the connected deck that receives it. */
		receivers?: Record<string, string>;
		/** Level of the heading of the commands, under the one of the step or dialog that holds the list. */
		headingLevel?: 3 | 4;
	}

	let {
		preview,
		names,
		selected = $bindable(),
		decks = [],
		receivers = $bindable({}),
		headingLevel = 3
	}: Props = $props();

	const uid = $props.id();
	const selectable = $derived(selected !== undefined);

	const applied = $derived(
		preview.sections.filter(
			(section) => selected === undefined || selected.includes(section.section)
		)
	);
	const commands = $derived(
		applied.flatMap((section) =>
			section.executables.map((item) => ({ section: section.section, item }))
		)
	);

	function toggle(id: string, checked: boolean) {
		if (!selected) return;
		selected = preview.sections
			.map((section) => section.section)
			.filter((one) => (one === id ? checked : selected?.includes(one)));
		const serial = deviceSerialOf(id);
		if (!checked && serial) delete receivers[serial];
	}

	/** Decks a device may be given to: not the ones another device takes, nor one the backup itself holds. */
	function offered(serial: string): readonly DeviceInfo[] {
		const taken = new Set(
			Object.entries(receivers)
				.filter(([from]) => from !== serial)
				.map(([, to]) => to)
		);
		const held = new Set(
			(selected ?? []).map(deviceSerialOf).filter((one): one is string => one !== undefined)
		);
		return decks.filter((deck) => !taken.has(deck.serial) && !held.has(deck.serial));
	}

	function receive(serial: string, target: string) {
		if (target) receivers[serial] = target;
		else delete receivers[serial];
	}

	function awayNote(id: string, receiver: string | undefined): string {
		if (receiver) return `Remplacera la configuration de ${names.get(receiver) ?? receiver}.`;
		return id.startsWith('devices/')
			? 'Stream Deck absent : sa configuration est gardée pour son prochain branchement.'
			: 'Souris absente : sa configuration est gardée pour son prochain branchement.';
	}
</script>

<div class="flex flex-col gap-4 text-sm">
	<ul
		aria-label="Sections de la sauvegarde"
		class="flex flex-col divide-y divide-line rounded-lg border border-line"
	>
		{#each preview.sections as section, index (section.section)}
			{@const serial = deviceSerialOf(section.section)}
			{@const receiver = serial ? receivers[serial] : undefined}
			<li class="flex flex-col gap-1 px-3 py-2">
				<div class="flex flex-wrap items-center gap-x-3 gap-y-1">
					{#if selected}
						<input
							id="{uid}-{index}"
							type="checkbox"
							class="size-5 accent-ring focus-ring"
							checked={selected.includes(section.section)}
							aria-describedby="{uid}-{index}-state"
							onchange={(event) => toggle(section.section, event.currentTarget.checked)}
						/>
					{/if}
					<label
						id="{uid}-{index}-name"
						for={selected ? `${uid}-${index}` : undefined}
						class="font-medium"
					>
						{sectionLabel(section.section, names)}
					</label>
					<span id="{uid}-{index}-state" class="rounded-md border border-edge px-1.5 text-xs">
						{CHANGE_LABELS[section.change]}
					</span>
				</div>
				{#if !section.present}
					<p class="text-muted">{awayNote(section.section, receiver)}</p>
					{#if selected && serial && selected.includes(section.section) && decks.length > 0}
						<label class="flex flex-wrap items-center gap-2 text-sm" for="{uid}-{index}-to">
							Appliquer à
							<select
								id="{uid}-{index}-to"
								class="field w-auto"
								aria-describedby="{uid}-{index}-name"
								value={receiver ?? ''}
								onchange={(event) => receive(serial, event.currentTarget.value)}
							>
								<option value="">Garder pour son prochain branchement</option>
								{#each offered(serial) as deck (deck.serial)}
									<option value={deck.serial}>{deck.displayName} ({deck.serial})</option>
								{/each}
							</select>
						</label>
					{/if}
				{/if}
			</li>
		{/each}
	</ul>

	{#if commands.length > 0}
		<div class="flex flex-col gap-2">
			<svelte:element this={`h${headingLevel}`} id="{uid}-commands" class="font-semibold">
				Ce que {selectable ? 'l’import' : 'la restauration'} fait exécuter ou atteindre sur cet ordinateur
			</svelte:element>
			<p class="text-muted">
				Vérifiez chaque ligne : un alias ou un réglage activé est exécuté dès le prochain shell ou
				la prochaine commande Git ou SSH. Un élément désactivé est importé, à un interrupteur d’être
				activé.
			</p>
			<!-- Scrolls when a command is long: reachable and named for the keyboard. -->
			<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
			<div class="overflow-x-auto" tabindex="0" role="region" aria-labelledby="{uid}-commands">
				<table class="w-full text-left" aria-labelledby="{uid}-commands">
					<thead>
						<tr class="border-b border-line text-muted">
							<th scope="col" class="py-1 pr-3 font-medium">Section</th>
							<th scope="col" class="py-1 pr-3 font-medium">Type</th>
							<th scope="col" class="py-1 pr-3 font-medium">Nom</th>
							<th scope="col" class="py-1 pr-3 font-medium">Commande ou valeur</th>
							<th scope="col" class="py-1 font-medium">État</th>
						</tr>
					</thead>
					<tbody>
						{#each commands as { section, item }, index (index)}
							<tr class="border-b border-line align-top last:border-0">
								<td class="py-1 pr-3">{sectionLabel(section, names)}</td>
								<td class="py-1 pr-3">{EXECUTABLE_LABELS[item.kind]}</td>
								<td class="py-1 pr-3 font-mono wrap-anywhere">{item.name}</td>
								<td class="py-1 pr-3 font-mono wrap-anywhere">{item.detail}</td>
								<td class="py-1">
									{item.enabled ? 'Activé' : 'Désactivé'}
									{#if item.takesOver}
										<br /><strong class="font-semibold">
											Reprend « {item.name} » déjà présent : la ligne d’origine est mise en commentaire.
										</strong>
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		</div>
	{/if}

	{#if preview.kept.length > 0}
		<p class="text-muted">
			Laissé tel quel, la sauvegarde n’en parle pas&nbsp;: {preview.kept
				.map((id) => sectionLabel(id, names))
				.join(', ')}.
		</p>
	{/if}
</div>
