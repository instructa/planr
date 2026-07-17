import defaultMdxComponents from 'fumadocs-ui/mdx';
import type { MDXComponents } from 'mdx/types';
import { CommandBlock } from '@/components/command-block';
import { PathCard } from '@/components/path-card';

export function getMDXComponents(components?: MDXComponents) {
  return {
    ...defaultMdxComponents,
    CommandBlock,
    PathCard,
    ...components,
  } satisfies MDXComponents;
}

export const useMDXComponents = getMDXComponents;

declare global {
  type MDXProvidedComponents = ReturnType<typeof getMDXComponents>;
}
