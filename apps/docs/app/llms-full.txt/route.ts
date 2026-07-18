import { getLLMFullText, markdownResponse } from '@/lib/llm';

export const dynamic = 'force-static';

export async function GET() {
  return markdownResponse(await getLLMFullText());
}
