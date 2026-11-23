<!--
@component
The aliases found in the startup files of the shell that HuBergamof does not manage. They are
only shown: « Intégrer » moves one into the configuration, switched on. Dashed lines and the file
they come from tell them from the managed ones. Once one is taken in, the focus goes to the next
« Intégrer » button, or to `fallbackFocus` when it was the last one.
-->
<script lang="ts">
	import { hasErrors, validateAlias } from '#lib/deck/alias-validation.js';
	import type { SystemAlias } from '#lib/generated/index.js';

	interface Props {
		aliases: readonly SystemAlias[];
		/** Whether the configuration is full: nothing can be added to it. */
		full: boolean;
		onadopt: (alias: SystemAlias) => void;
		/** Where the focus goes when there is no other alias to take in. */
		fallbackFocus?: () => void;
	}

	let { aliases, full, onadopt, fallbackFocus }: Props = $props();

	let list: HTMLUListElement | undefined = $state();

	/** The button of an alias, found by what identifies it. */
	function adoptButton(key: string): HTMLButtonElement | undefined {
		return [...(list?.querySelectorAll<HTMLButtonElement>('button[data-adopt]') ?? [])].find(
			(button) => button.dataset.adopt === key
		);
	}

	function keyOf({ source, name }: SystemAlias): string {
		return `${source}:${name}`;
	}

	/** The alias leaves the list: its neighbours stay, so the focus can move before it is gone. */
	function adopt(alias: SystemAlias, index: number) {
		const next = aliases[index + 1] ?? aliases[index - 1];
		onadopt(alias);
		const button = next ? adoptButton(keyOf(next)) : undefined;
		if (button) button.focus();
		else fallbackFocus?.();
	}

	const uid = $props.id();

	/** Why an alias cannot be taken in, if it cannot: the manager would refuse it. */
	function obstacle(alias: SystemAlias): string | undefined {
		if (full) return 'La configuration est pleine.';
		return hasErrors(validateAlias(alias, []))
			? 'Cet alias ne respecte pas les règles de HuBergamof (nom ou commande).'
			: undefined;
	}
</script>

{#if aliases.length === 0}
	<p class="text-sm text-muted">Aucun autre alias sur le système.</p>
{:else}
	<ul bind:this={list} class="flex flex-col gap-2" aria-label="Alias du système, non gérés">
		{#each aliases as alias, index (keyOf(alias))}
			{@const reason = obstacle(alias)}
			<li
				class="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border border-dashed border-muted bg-sunken p-3"
			>
				<div class="flex min-w-0 flex-1 basis-60 flex-col">
					<code class="font-mono font-semibold break-all">{alias.name}</code>
					<code class="font-mono text-sm break-all">{alias.command}</code>
					<span class="text-sm text-muted">Non géré · lu dans ~/{alias.source}</span>
				</div>
				<div class="flex flex-col items-end gap-1">
					<button
						type="button"
						data-adopt={keyOf(alias)}
						class="secondary-button"
						aria-label="Intégrer l’alias {alias.name}"
						aria-disabled={reason !== undefined}
						aria-describedby={reason ? `${uid}-reason-${index}` : undefined}
						onclick={() => !reason && adopt(alias, index)}
					>
						Intégrer
					</button>
					{#if reason}
						<span id="{uid}-reason-{index}" class="text-sm text-muted">{reason}</span>
					{/if}
				</div>
			</li>
		{/each}
	</ul>
{/if}
