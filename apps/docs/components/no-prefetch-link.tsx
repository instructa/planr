'use client';

import NextLink from 'next/link';
import type { Framework } from 'fumadocs-core/framework';

export const NoPrefetchLink: NonNullable<Framework['Link']> = ({ href, ...props }) => {
  if (!href) return <a {...props} />;
  return <NextLink {...props} href={href} prefetch={false} />;
};
