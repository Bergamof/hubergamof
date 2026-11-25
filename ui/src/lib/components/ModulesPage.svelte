<!--
@component
The modules of the application that have settings: a list, and the panel of the one chosen. The one chosen is
named by the address (`?module=`), else the first. A module without any setting is not listed: what it says
and does is on the home page.
-->
<script lang="ts">
	import { getManager } from '#lib/deck/manager-context.js';
	import { moduleName } from '#lib/deck/display.js';
	import { moduleHref } from '#lib/deck/nav-model.js';
	import EmptyState from './EmptyState.svelte';
	import Icon, { type IconName } from './Icon.svelte';
	import ModuleDetail from './ModuleDetail.svelte';

	interface Props {
		/** The module the address names (`?module=`), if any. */
		requested: string | null;
	}

	let { requested }: Props = $props();

	const { deck, api } = getManager();

	const MODULE_ICON: Record<string, IconName> = {
		pomodoro: 'timer',
		audio: 'volume',
		teams: 'chat',
		intellij: 'code',
		claude: 'spark'
	};

	/** The modules that can be set: the catalog says which settings each one has. */
	const configurable = $derived(
		deck.application.modules.filter((view) => (deck.moduleOf(view.kind)?.settings.length ?? 0) > 0)
	);
	const chosen = $derived(configurable.find((view) => view.kind === requested) ?? configurable[0]);
</script>

{#if configurable.length === 0}
	<EmptyState title="Aucun module à régler">
		<p>Les modules sans réglage ne sont pas listés : leur état se lit sur l’accueil.</p>
	</EmptyState>
{:else}
	<div class="grid grid-cols-[16rem_minmax(0,1fr)] items-start gap-4">
		<nav aria-label="Modules à régler" class="card p-1.5">
			<ul class="flex flex-col gap-0.5">
				{#each configurable as view (view.kind)}
					{@const current = view.kind === chosen?.kind}
					<li>
						<a
							href={moduleHref(view.kind)}
							data-sveltekit-preload-data="off"
							data-sveltekit-noscroll
							data-sveltekit-keepfocus
							aria-current={current ? 'true' : undefined}
							class={[
								'flex min-h-10 flex-col justify-center rounded-lg px-2.5 py-1 text-fg no-underline focus-ring hover:bg-sunken',
								current && 'bg-tint'
							]}
						>
							<span class={['flex items-center gap-2', current ? 'font-semibold' : 'font-medium']}>
								<Icon name={MODULE_ICON[view.kind] ?? 'modules'} />
								{moduleName(view.kind, deck.moduleOf(view.kind))}
							</span>
							{#if view.status}
								<span class="truncate pl-6 text-xs text-muted">{view.status.split('\n')[0]}</span>
							{/if}
						</a>
					</li>
				{/each}
			</ul>
		</nav>
		{#if chosen}
			<!-- One editor per module: its form is filled once, from the settings of the module it was made for. -->
			{#key chosen.kind}
				<ModuleDetail {deck} {api} view={chosen} />
			{/key}
		{/if}
	</div>
{/if}
