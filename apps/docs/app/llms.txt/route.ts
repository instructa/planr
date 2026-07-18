import { getLLMIndex, markdownResponse } from '@/lib/llm';

export const dynamic = 'force-static';

export function GET() {
  return markdownResponse(getLLMIndex());
}
