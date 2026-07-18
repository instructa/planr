import type { MetadataRoute } from 'next';

export const dynamic = 'force-static';
import { source } from '@/lib/source';

export default function sitemap(): MetadataRoute.Sitemap {
  const origin = process.env.NEXT_PUBLIC_SITE_URL ?? 'http://localhost:3000';
  const pages = source.getPages().map((page) => ({
    url: `${origin}${page.url}`,
    changeFrequency: 'weekly' as const,
    priority: page.url === '/docs' ? 0.9 : 0.7,
  }));

  return [
    { url: origin, changeFrequency: 'weekly', priority: 1 },
    ...pages,
  ];
}
