<script lang="ts">
	import HeadsetPage from '#lib/components/HeadsetPage.svelte';
	import { getManager } from '#lib/deck/manager-context.js';

	const { deck } = getManager();
</script>

<HeadsetPage {deck} />
