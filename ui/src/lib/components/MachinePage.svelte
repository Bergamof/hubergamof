<!--
@component
The machine: what HuBergamof sets up on the computer of the person. A choice at the top picks the
topic (the shell aliases, Git, SSH). All the topics stay in the page, so what is typed in one survives a look
at the other; they all read the computer when the page opens, and never watch it.
-->
<script lang="ts">
	import type { DeckState } from '#lib/deck/deck-state.svelte.js';
	import GitPanel from './GitPanel.svelte';
	import SegmentedControl from './SegmentedControl.svelte';
	import ShellAliasesPanel from './ShellAliasesPanel.svelte';
	import SshPanel from './SshPanel.svelte';

	interface Props {
		deck: DeckState;
	}

	let { deck }: Props = $props();

	type Topic = 'aliases' | 'git' | 'ssh';
	const TOPICS: { value: Topic; label: string }[] = [
		{ value: 'aliases', label: 'Alias du shell' },
		{ value: 'git', label: 'Git' },
		{ value: 'ssh', label: 'SSH' }
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
	<!-- Both stay in the page: what is typed in one is still there when the other has been looked at. -->
	<div hidden={topic !== 'aliases'}>
		<ShellAliasesPanel {deck} />
	</div>
	<div hidden={topic !== 'git'}>
		<GitPanel {deck} />
	</div>
	<div hidden={topic !== 'ssh'}>
		<SshPanel {deck} />
	</div>
</div>
