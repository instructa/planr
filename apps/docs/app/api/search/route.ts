import { createFromSource } from 'fumadocs-core/search/server';
import { deterministicSource } from '@/lib/source';

export const dynamic = 'force-static';

export const { staticGET: GET } = createFromSource(deterministicSource);
