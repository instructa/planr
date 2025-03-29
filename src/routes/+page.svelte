<script lang="ts">
	import { generateImage } from '$lib/services/imageService';
	import ImageDisplay from '$lib/components/ImageDisplay.svelte';
	import PromptInput from '$lib/components/PromptInput.svelte';

	let imageUrl = '';
	let error = '';
	let isLoading = false;

	async function handleGenerateImage(prompt: string) {
		error = '';
		isLoading = true;

		try {
			imageUrl = await generateImage(prompt);
		} catch (err) {
			const errorMessage = err instanceof Error ? err.message : 'Failed to generate image';
			error = errorMessage;
			console.error('Error generating image:', err);
		} finally {
			isLoading = false;
		}
	}
</script>

<div class="container mx-auto max-w-3xl px-4 py-8">
	<header class="mb-8 text-center">
		<h1 class="mb-2 text-3xl font-bold">AI Image Generator</h1>
		<p class="text-gray-600">Enter a text prompt to generate an image</p>
	</header>

	<main>
		<ImageDisplay {imageUrl} />

		{#if error}
			<div
				class="mb-4 rounded border border-red-400 bg-red-100 px-4 py-3 text-red-700"
				role="alert"
			>
				<p>{error}</p>
			</div>
		{/if}

		<PromptInput onSubmit={handleGenerateImage} />

		{#if isLoading}
			<div class="mt-4 text-center text-gray-600">
				<p>Generating your image, please wait...</p>
			</div>
		{/if}
	</main>
</div>
