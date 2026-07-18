import defaultMdxComponents from 'fumadocs-ui/mdx';
import type { MDXComponents } from 'mdx/types';
import { AgentRecipe } from '@/components/agent-recipe';
import { CommandBlock } from '@/components/command-block';
import { PathCard } from '@/components/path-card';
import { PromptBlock } from '@/components/prompt-block';

export function getMDXComponents(components?: MDXComponents) {
  return {
    ...defaultMdxComponents,
    AgentRecipe,
    CommandBlock,
    PathCard,
    PromptBlock,
    ...components,
  } satisfies MDXComponents;
}

export const useMDXComponents = getMDXComponents;

declare global {
  type MDXProvidedComponents = ReturnType<typeof getMDXComponents>;
}
