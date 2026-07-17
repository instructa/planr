import type { MetadataRoute } from 'next';

export default function manifest(): MetadataRoute.Manifest {
  return {
    name: 'Planr Documentation',
    short_name: 'Planr Docs',
    description: 'Documentation for Planr local-first coding-agent coordination.',
    start_url: '/docs',
    display: 'standalone',
    background_color: '#0a1210',
    theme_color: '#176b59',
  };
}
