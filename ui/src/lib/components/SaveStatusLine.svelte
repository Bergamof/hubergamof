<!--
@component
The line that says whether the last change was saved, kept in view while the page scrolls (changes are made far
from the top), and its twin for screen readers.
-->
<script lang="ts">
	import { SAVE_STATUS_TEXT } from '#lib/deck/save-tracker.svelte.js';
	import type { SaveTracker } from '#lib/deck/save-tracker.svelte.js';

	let { tracker }: { tracker: SaveTracker } = $props();
</script>

<div class="sticky top-12 z-10 -mx-1 bg-page px-1 py-1">
	<p class="min-h-6 text-sm text-muted" aria-hidden="true">{SAVE_STATUS_TEXT[tracker.status]}</p>
	<p role="status" class="sr-only">{tracker.spoken}</p>
</div>
