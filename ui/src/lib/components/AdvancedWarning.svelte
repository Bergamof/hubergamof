<!--
@component
What the person must know before writing a setting of their own: it is written as it is, whatever it
does. Kept with the form that adds a setting (which it describes for screen readers), so that it is
read where it matters, not only at the top of the page.
-->
<script lang="ts">
	interface Props {
		id: string;
		/** The program that reads the setting. */
		tool: 'ssh' | 'git';
	}

	let { id, tool }: Props = $props();
</script>

<p {id} class="rounded-xl border border-line bg-sunken p-3 text-sm">
	<strong>Attention :</strong>
	{#if tool === 'ssh'}
		une option est écrite telle quelle, sans autre contrôle que sa forme. Certaines lancent un
		programme (<code class="font-mono">ProxyCommand</code>,
		<code class="font-mono">LocalCommand</code>, un <code class="font-mono">Match exec</code> d’un
		fichier inclus…) ou affaiblissent la connexion (<code class="font-mono"
			>StrictHostKeyChecking no</code
		>) : n’ajoutez que ce que vous comprenez. Avant d’appliquer, ssh lit le fichier. S’il refuse une
		option, la configuration reste enregistrée mais n’est pas appliquée, et un message le dit :
		corrigez l’option ou supprimez-la, sinon plus aucune modification SSH n’est appliquée à
		l’ordinateur.
	{:else}
		une option est écrite telle quelle, sans autre contrôle que sa forme. Certaines clés font
		exécuter un programme à Git (<code class="font-mono">core.pager</code>,
		<code class="font-mono">core.sshCommand</code>, <code class="font-mono">diff.tool</code>…) :
		n’ajoutez que ce que vous comprenez. N’y mettez ni jeton ni mot de passe : la valeur est
		affichée en clair, préférez une aide d’identification. Les alias (<code class="font-mono"
			>alias.*</code
		>) et les inclusions ont leur propre place et sont refusés ici.
	{/if}
</p>
