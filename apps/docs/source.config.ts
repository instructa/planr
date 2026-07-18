import { defineConfig, defineDocs } from 'fumadocs-mdx/config';
import { remarkLLMs } from 'fumadocs-core/mdx-plugins/remark-llms';

export const docs = defineDocs({
  dir: 'content/docs',
});

export default defineConfig({
  mdxOptions: {
    remarkPlugins: [
      [
        remarkLLMs,
        {
          mdxAsPlaceholder: [
            'AgentRecipe',
            'Callout',
            'Card',
            'Cards',
            'CommandBlock',
            'PathCard',
            'PromptBlock',
          ],
        },
      ],
    ],
  },
});
