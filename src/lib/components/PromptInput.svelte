<script lang="ts">
	let { onSubmit } = $props<{
		onSubmit: (prompt: string) => void;
	}>();

	let prompt = '';
	let isSubmitting = false;

	function handleSubmit() {
		if (!prompt.trim()) return;
		isSubmitting = true;
		onSubmit(prompt);
	}

	// Reset submitting state when component is updated externally
	$effect(() => {
		isSubmitting = false;
	});
</script>

<div class="w-full">
	<form on:submit|preventDefault={handleSubmit} class="flex flex-col gap-2">
		<label for="prompt-input" class="sr-only">Enter your text prompt</label>
		<textarea
			id="prompt-input"
			bind:value={prompt}
			placeholder="Enter a text prompt (e.g., 'cat in a hat')"
			class="min-h-[100px] w-full resize-y rounded-lg border border-gray-300 p-3 focus:border-blue-500 focus:ring-2 focus:ring-blue-500"
			disabled={isSubmitting}
		></textarea>
		<button
			type="submit"
			disabled={isSubmitting || !prompt.trim()}
			class="rounded-lg bg-blue-600 px-4 py-2 text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-50"
		>
			{isSubmitting ? 'Generating...' : 'Generate Image'}
		</button>
	</form>
</div>
