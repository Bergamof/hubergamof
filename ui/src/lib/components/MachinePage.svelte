<!--
@component
The machine: what HuBergamof sets up on the computer of the person. A choice at the top picks the
topic (the shell aliases, Git); each topic reads the computer when it is shown, never watches it.
-->
<script lang="ts">
	import type { DeckState } from '#lib/deck/deck-state.svelte.js';
	import GitPanel from './GitPanel.svelte';
	import SegmentedControl from './SegmentedControl.svelte';
	import ShellAliasesPanel from './ShellAliasesPanel.svelte';

	interface Props {
		deck: DeckState;
	}

	let { deck }: Props = $props();

	type Topic = 'aliases' | 'git';
	const TOPICS: { value: Topic; label: string }[] = [
		{ value: 'aliases', label: 'Alias du shell' },
		{ value: 'git', label: 'Git' }
	];

	let topic: Topic = $state('aliases');
</script>

<div class="flex flex-col gap-6">
	<SegmentedControl
		label="Rubrique de la machine"
		options={TOPICS}
		value={topic}
		onchange={(chosen) => (topic = chosen)}
	/>
	{#if topic === 'git'}
		<GitPanel {deck} />
	{:else}
		<ShellAliasesPanel {deck} />
	{/if}
</div>
