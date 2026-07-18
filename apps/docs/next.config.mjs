import { initOpenNextCloudflareForDev } from '@opennextjs/cloudflare';
import { createMDX } from 'fumadocs-mdx/next';
import { legacyRedirects } from './redirects.mjs';

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  redirects: async () => legacyRedirects,
  rewrites: async () => [
    {
      source: '/docs/:slug*.md',
      destination: '/api/markdown/:slug*',
    },
  ],
};

const withMDX = createMDX();

export default withMDX(config);

initOpenNextCloudflareForDev();
