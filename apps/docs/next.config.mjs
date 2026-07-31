import { createMDX } from 'fumadocs-mdx/next';

/** @type {import('next').NextConfig} */
const config = {
  output: 'export',
  reactStrictMode: true,
  generateBuildId: async () => 'planr-docs-static',
};

const withMDX = createMDX();

export default withMDX(config);
