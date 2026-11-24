<!--
@component
The hosts found in `~/.ssh/config` that HuBergamof does not manage. They are only shown: « Reprendre »
moves one into the configuration, switched on, and its section is commented out where it was. The other
options of a host become its advanced options; one that cannot be carried without loss keeps the host from being taken: the reason is given. Dashed lines tell
them from the managed ones. Once one is taken, the focus goes to the next « Reprendre » button, or to
`fallbackFocus` when it was the last one.
-->
<script lang="ts">
	import { takeOverObstacle } from '#lib/deck/ssh-validation.js';
	import type { SshSystemHost } from '#lib/generated/index.js';

	interface Props {
		hosts: readonly SshSystemHost[];
		/** The aliases of the managed hosts. */
		takenAliases: readonly string[];
		/** Whether the configuration is full: nothing can be added to it. */
		full: boolean;
		onadopt: (host: SshSystemHost) => void;
		/** Where the focus goes when there is no other host to take. */
		fallbackFocus?: () => void;
	}

	let { hosts, takenAliases, full, onadopt, fallbackFocus }: Props = $props();

	const uid = $props.id();
	let list: HTMLUListElement | undefined = $state();

	function adoptButton(alias: string): HTMLButtonElement | undefined {
		return [...(list?.querySelectorAll<HTMLButtonElement>('button[data-adopt]') ?? [])].find(
			(button) => button.dataset.adopt === alias
		);
	}

	/** The host leaves the list: its neighbours stay, so the focus can move before it is gone. */
	function adopt(host: SshSystemHost, index: number) {
		const next = hosts[index + 1] ?? hosts[index - 1];
		onadopt(host);
		const button = next ? adoptButton(next.alias) : undefined;
		if (button) button.focus();
		else fallbackFocus?.();
	}

	function obstacle(host: SshSystemHost): string | undefined {
		if (full) return 'La configuration est pleine.';
		return takeOverObstacle(host, takenAliases);
	}

	/** What the host stands for, in words. */
	function summary(host: SshSystemHost): string {
		const target = `${host.user ? `${host.user}@` : ''}${host.hostName || host.alias}${host.port === null ? '' : `:${host.port}`}`;
		return [
			target,
			host.proxyJump && `via ${host.proxyJump}`,
			host.forwardAgent && 'agent transmis',
			host.options.length > 0 &&
				`${host.options.length} option${host.options.length > 1 ? 's' : ''} avancée${host.options.length > 1 ? 's' : ''}`
		]
			.filter(Boolean)
			.join(' · ');
	}
</script>

{#if hosts.length === 0}
	<p class="text-sm text-muted">Aucun autre serveur dans ~/.ssh/config.</p>
{:else}
	<ul
		bind:this={list}
		class="flex flex-col gap-2"
		aria-label="Serveurs SSH définis ailleurs, non gérés"
	>
		{#each hosts as host, index (host.alias)}
			{@const reason = obstacle(host)}
			<li
				class="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border border-dashed border-muted bg-sunken p-3"
			>
				<div class="flex min-w-0 flex-1 basis-60 flex-col">
					<code class="font-mono font-semibold break-all">{host.alias}</code>
					<span class="text-sm break-all">{summary(host)}</span>
					<span class="text-sm text-muted">Non géré</span>
				</div>
				<div class="flex flex-col items-end gap-1">
					<button
						type="button"
						data-adopt={host.alias}
						class="secondary-button"
						aria-label="Reprendre le serveur {host.alias}"
						aria-disabled={reason !== undefined}
						aria-describedby={reason ? `${uid}-reason-${index}` : undefined}
						onclick={() => !reason && adopt(host, index)}
					>
						Reprendre
					</button>
					{#if reason}
						<span id="{uid}-reason-{index}" class="max-w-80 text-right text-sm text-muted">
							{reason}
						</span>
					{/if}
				</div>
			</li>
		{/each}
	</ul>
{/if}
