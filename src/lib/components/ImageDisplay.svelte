<script lang="ts">
	let { imageUrl = '' } = $props();
</script>

{#if imageUrl}
	<div class="relative mb-4 h-64 w-full overflow-hidden rounded-lg bg-gray-100 md:h-80">
		<img
			src={imageUrl}
			alt="AI-generated image from your prompt"
			class="h-full w-full object-contain"
		/>
	</div>
{:else}
	<div class="mb-4 flex h-64 w-full items-center justify-center rounded-lg bg-gray-100 md:h-80">
		<p class="text-gray-500">Your generated image will appear here</p>
	</div>
{/if}
