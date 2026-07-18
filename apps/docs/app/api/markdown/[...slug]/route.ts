import { notFound } from 'next/navigation';
import { getLLMText, markdownResponse } from '@/lib/llm';
import { source } from '@/lib/source';

type RouteProps = {
  params: Promise<{ slug: string[] }>;
};

export const dynamicParams = false;

export async function GET(_request: Request, { params }: RouteProps) {
  const { slug } = await params;
  const page = source.getPage(slug.length === 1 && slug[0] === 'index' ? [] : slug);
  if (!page) notFound();
  return markdownResponse(await getLLMText(page));
}

export function generateStaticParams() {
  return source.getPages().map((page) => ({ slug: page.slugs.length ? page.slugs : ['index'] }));
}
