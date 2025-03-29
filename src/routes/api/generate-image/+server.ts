import { json } from '@sveltejs/kit';
import type { RequestEvent } from '@sveltejs/kit';
import { fal } from '@fal-ai/client';

// Define Flux-schnell model ID
const MODEL_ID = 'fal-ai/flux-schnell';

export async function POST({ request }: RequestEvent) {
	try {
		// Validate API key
		if (!process.env.FAL_AI_API_KEY) {
			return json({ error: 'API key not configured' }, { status: 500 });
		}

		// Parse request body to get prompt
		const { prompt } = await request.json();

		// Validate prompt
		if (!prompt || typeof prompt !== 'string' || !prompt.trim()) {
			return json({ error: 'Please enter a valid prompt' }, { status: 400 });
		}

		// Set up FAL AI client with API key
		fal.config({
			credentials: process.env.FAL_AI_API_KEY
		});

		// Call fal.ai API to generate image
		const result = await fal.run(MODEL_ID, {
			input: {
				prompt: prompt.trim(),
				num_inference_steps: 30
			}
		});

		// Extract image URL from response
		if (!result.data || !result.data.images || !result.data.images[0]?.url) {
			return json({ error: 'Failed to generate image' }, { status: 500 });
		}

		// Return successful response with image URL
		return json({ image_url: result.data.images[0].url });
	} catch (error: unknown) {
		console.error('Error generating image:', error);

		// Type guard for error with message property
		const errorWithMessage = error as { message?: string };

		// Handle different error types
		if (errorWithMessage.message?.includes('rate limit')) {
			return json({ error: 'Rate limit reached, please try again later' }, { status: 429 });
		}

		if (
			errorWithMessage.message?.includes('budget exceeded') ||
			errorWithMessage.message?.includes('credit')
		) {
			return json({ error: 'Daily budget exceeded, service unavailable' }, { status: 402 });
		}

		// Generic error response
		return json({ error: 'An error occurred, please try again' }, { status: 500 });
	}
}
