<!--
@component
What keeps the page from being used: the computer is locked, or the connection to the manager is lost.
The page stays readable and the navigation stays usable; only the controls are disabled. The lock
comes first when both hold.
-->
<script lang="ts">
	import Icon from './Icon.svelte';

	interface Props {
		locked: boolean;
		online: boolean;
	}

	let { locked, online }: Props = $props();
</script>

<div role="status">
	{#if locked}
		<p class="flex items-center gap-2.5 border-b border-line bg-warning-soft px-6 py-2.5 text-sm">
			<Icon name="lock" class="size-4 text-warning" />
			<span>
				<strong>Ordinateur verrouillé.</strong> Configuration en lecture seule jusqu’au déverrouillage.
			</span>
		</p>
	{:else if !online}
		<p class="flex items-center gap-2.5 border-b border-line bg-danger-soft px-6 py-2.5 text-sm">
			<Icon name="wifiOff" class="size-4 text-danger" />
			<span>
				<strong>Connexion au gestionnaire perdue.</strong> Reconnexion en cours : la configuration affichée
				peut ne pas être à jour.
			</span>
		</p>
	{/if}
</div>
