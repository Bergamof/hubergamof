<!--
@component
What an import or a restore did to each section: in words, with the manager's own explanation
(English) for what it refused or could not put on the computer.
-->
<script lang="ts">
	import type { ImportReport } from '#lib/generated/index.js';
	import { outcomeText, sectionLabel } from '#lib/deck/backup-labels.js';

	interface Props {
		report: ImportReport;
		/** Name of each connected device and mouse, by serial. */
		names: ReadonlyMap<string, string>;
	}

	let { report, names }: Props = $props();
</script>

<ul class="flex flex-col divide-y divide-line rounded-lg border border-line text-sm">
	{#each report.sections as { section, outcome } (section)}
		{@const text = outcomeText(outcome)}
		<li class="flex flex-col gap-0.5 px-3 py-2">
			<span class="font-medium">{sectionLabel(section, names)}</span>
			<span>{text.label}</span>
			{#if text.detail}
				<span class="text-muted">Détail&nbsp;: <span lang="en">{text.detail}</span></span>
			{/if}
		</li>
	{/each}
</ul>
