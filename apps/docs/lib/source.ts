import { docs } from 'collections/server';
import { loader } from 'fumadocs-core/source';

export const source = loader({
  baseUrl: '/docs',
  source: docs.toFumadocsSource(),
});

export type SourcePage = ReturnType<typeof source.getPages>[number];

export function canonicalPathForPage(page: SourcePage) {
  return page.slugs.length ? `/docs/${page.slugs.join('/')}` : '/docs';
}

export function getSortedPages() {
  return [...source.getPages()].sort((left, right) =>
    canonicalPathForPage(left).localeCompare(canonicalPathForPage(right)),
  );
}

export const deterministicSource = {
  ...source,
  getPages: getSortedPages,
};
