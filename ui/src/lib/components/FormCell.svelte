<!--
@component
A cell of a `FormGrid`: the number of columns it takes (of twelve) once the form is wide enough, the whole row
before. Short values (a port, a number) take 2 or 3, names 4 or 5, paths and lists 8 or more.
-->
<script lang="ts" module>
	export type Span = 2 | 3 | 4 | 5 | 6 | 7 | 8 | 12;

	/** Written out in full: Tailwind finds a class by reading it, not by building it. */
	const SPANS: Readonly<Record<Span, string>> = {
		2: '@xl:col-span-2',
		3: '@xl:col-span-3',
		4: '@xl:col-span-4',
		5: '@xl:col-span-5',
		6: '@xl:col-span-6',
		7: '@xl:col-span-7',
		8: '@xl:col-span-8',
		12: '@xl:col-span-12'
	};

	export const spanClass = (span: Span): string => SPANS[span];
</script>

<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		span?: Span;
		children: Snippet;
	}

	let { span = 12, children }: Props = $props();
</script>

<div class={['col-span-12 min-w-0', spanClass(span)]}>
	{@render children()}
</div>
