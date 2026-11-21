<!--
@component
A row of tabs (the tab list only; the parent shows the panel, with the ids given by {@link panelId} and
{@link tabId}). Arrow keys, Home and End move between the tabs, which are selected as they are reached.
-->
<script lang="ts" module>
	/** Id of the tab `id` of the tab list whose `prefix` is given. */
	export const tabId = (prefix: string, id: string) => `${prefix}-tab-${id}`;
	/** Id of the panel `id` of the tab list whose `prefix` is given. */
	export const panelId = (prefix: string, id: string) => `${prefix}-panel-${id}`;
</script>

<script lang="ts" generics="T extends string">
	interface Props {
		/** What the tabs are about, read by assistive technologies. */
		label: string;
		tabs: readonly { id: T; label: string }[];
		selected: T;
		/** Makes the ids of the tabs and of their panels unique on the page. */
		prefix: string;
		onselect: (id: T) => void;
	}

	let { label, tabs, selected, prefix, onselect }: Props = $props();

	let list: HTMLDivElement | undefined = $state();

	function keydown(event: KeyboardEvent) {
		const index = tabs.findIndex((tab) => tab.id === selected);
		const target = {
			ArrowRight: (index + 1) % tabs.length,
			ArrowLeft: (index - 1 + tabs.length) % tabs.length,
			Home: 0,
			End: tabs.length - 1
		}[event.key];
		if (target === undefined) return;
		event.preventDefault();
		onselect(tabs[target].id);
		list?.querySelector<HTMLElement>(`#${CSS.escape(tabId(prefix, tabs[target].id))}`)?.focus();
	}
</script>

<div
	bind:this={list}
	role="tablist"
	aria-label={label}
	tabindex="-1"
	class="flex flex-wrap gap-1 border-b border-line bg-sunken px-3.5 pt-2.5"
	onkeydown={keydown}
>
	{#each tabs as tab (tab.id)}
		<button
			type="button"
			role="tab"
			id={tabId(prefix, tab.id)}
			aria-selected={selected === tab.id}
			aria-controls={panelId(prefix, tab.id)}
			tabindex={selected === tab.id ? 0 : -1}
			class={[
				'rounded-t-[9px] px-3.5 py-2.5 text-sm font-semibold focus-ring',
				selected === tab.id
					? 'bg-surface text-fg shadow-[inset_0_2px_0_var(--color-accent)]'
					: 'text-muted hover:text-fg'
			]}
			onclick={() => onselect(tab.id)}
		>
			{tab.label}
		</button>
	{/each}
</div>
