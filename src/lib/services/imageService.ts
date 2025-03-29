/**
 * Service for generating images via the fal.ai API
 */

interface GenerateImageResponse {
	image_url: string;
}

interface GenerateImageError {
	error: string;
}

/**
 * Generates an image using the provided text prompt
 */
export async function generateImage(prompt: string): Promise<string> {
	try {
		const response = await fetch('/api/generate-image', {
			method: 'POST',
			headers: {
				'Content-Type': 'application/json'
			},
			body: JSON.stringify({ prompt })
		});

		if (!response.ok) {
			const errorData = (await response.json()) as GenerateImageError;
			throw new Error(errorData.error || 'Failed to generate image');
		}

		const data = (await response.json()) as GenerateImageResponse;
		return data.image_url;
	} catch (error) {
		console.error('Error generating image:', error);
		throw error;
	}
}
