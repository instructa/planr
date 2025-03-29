import { json } from '@sveltejs/kit';
import { env } from '$env/dynamic/private';

export async function GET() {
	// Check if API key exists (return masked version for security)
	const apiKey = env.FAL_AI_API_KEY || process.env.FAL_AI_API_KEY;

	return json({
		hasApiKey: !!apiKey,
		// Only show first few characters for security
		keyPreview: apiKey ? `${apiKey.substring(0, 8)}...` : null,
		envVars: {
			// List all env variables without revealing actual values
			names: Object.keys(env).filter((key) => !key.includes('SECRET') && !key.includes('KEY')),
			// Check specifically for our API key
			hasFalApiKey: 'FAL_AI_API_KEY' in env
		}
	});
}
