import type { MetadataRoute } from 'next';

export const dynamic = 'force-static';

export default function robots(): MetadataRoute.Robots {
  const origin = process.env.NEXT_PUBLIC_SITE_URL ?? 'http://localhost:3000';

  return {
    rules: { userAgent: '*', allow: '/' },
    sitemap: `${origin}/sitemap.xml`,
  };
}
