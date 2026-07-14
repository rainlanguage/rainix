<script lang="ts">
	let { items }: { items: string[] } = $props();
</script>

{#snippet row(label)}
	<li class='item'>{label}</li>
{/snippet}

<ul>
	{#each items as item}
		{@render row(item)}
	{/each}
</ul>
