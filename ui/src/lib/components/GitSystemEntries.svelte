<!--
@component
The values of the Git configuration that HuBergamof manages and that Git also finds in other files
of the computer. They are only shown: HuBergamof's own come last and win. Dashed lines and the file
they come from tell them from what is managed.
-->
<script lang="ts">
	import type { GitSystemEntry } from '#lib/generated/index.js';

	interface Props {
		entries: readonly GitSystemEntry[];
		/** `git` could not be run: nothing was read, which is not the same as nothing found. */
		unreadable?: boolean;
	}

	let { entries, unreadable = false }: Props = $props();
</script>

{#if unreadable}
	<p class="text-sm text-muted">Non lu : le programme git est introuvable.</p>
{:else if entries.length === 0}
	<p class="text-sm text-muted">Aucune autre valeur gérée par HuBergamof n’est définie ailleurs.</p>
{:else}
	<ul class="flex flex-col gap-2" aria-label="Valeurs de Git définies ailleurs">
		{#each entries as entry, index (`${entry.origin}:${entry.key}:${index}`)}
			<li class="flex flex-col rounded-xl border border-dashed border-muted bg-sunken p-3">
				<code class="font-mono font-semibold break-all">{entry.key}</code>
				<code class="font-mono text-sm break-all">{entry.value}</code>
				<span class="text-sm text-muted">Non géré · lu dans {entry.origin}</span>
			</li>
		{/each}
	</ul>
{/if}
