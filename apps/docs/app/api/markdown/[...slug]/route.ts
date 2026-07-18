import { notFound } from 'next/navigation';
import { getLLMText, markdownResponse } from '@/lib/llm';
import { source } from '@/lib/source';

type RouteProps = {
  params: Promise<{ slug: string[] }>;
};

export const dynamicParams = false;

export async function GET(_request: Request, { params }: RouteProps) {
  const { slug } = await params;
  const filename = slug.at(-1);
  if (!filename?.endsWith('.md')) notFound();
  const pageSlug = [...slug];
  pageSlug[pageSlug.length - 1] = filename.slice(0, -3);
  const page = source.getPage(pageSlug.length === 1 && pageSlug[0] === 'index' ? [] : pageSlug);
  if (!page) notFound();
  return markdownResponse(await getLLMText(page));
}

export function generateStaticParams() {
  return source.getPages().map((page) => {
    const slug = page.slugs.length ? [...page.slugs] : ['index'];
    slug[slug.length - 1] = `${slug.at(-1)}.md`;
    return { slug };
  });
}
