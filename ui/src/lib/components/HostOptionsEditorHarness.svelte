<!--
@component
Test harness for `HostOptionsEditor`: the editor in a form, over a list kept in a variable as the form of
a host keeps its draft. `submitted` counts the submissions of the form.
-->
<script lang="ts">
	import type { AdvancedSetting } from '#lib/generated/index.js';
	import HostOptionsEditor from './HostOptionsEditor.svelte';

	interface Props {
		initial?: AdvancedSetting[];
		host?: string;
		/** Called with the options each time they change. */
		onoptions?: (options: AdvancedSetting[]) => void;
		/** Called with what `commit()` answered. */
		oncommit?: (answer: boolean) => void;
	}

	let { initial = [], host = '', onoptions, oncommit }: Props = $props();

	// svelte-ignore state_referenced_locally
	let options = $state(initial);
	let submitted = $state(0);
	let editor: ReturnType<typeof HostOptionsEditor> | undefined = $state();
</script>

<form
	novalidate
	aria-label="Hôte"
	onsubmit={(event) => {
		event.preventDefault();
		submitted += 1;
	}}
>
	<p id="title">Options avancées</p>
	<HostOptionsEditor
		bind:this={editor}
		{options}
		{host}
		labelledby="title"
		onchange={(next) => {
			options = next;
			onoptions?.(next);
		}}
	/>
	<button type="submit">Envoyer</button>
	<button type="button" onclick={() => oncommit?.(editor?.commit() ?? true)}>Valider</button>
	<output aria-label="Envois">{submitted}</output>
</form>
