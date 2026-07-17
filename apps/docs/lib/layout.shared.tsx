import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { PlanrMark } from '@/components/planr-mark';

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: (
        <span className="inline-flex items-center gap-2.5 font-semibold tracking-tight">
          <PlanrMark className="size-7" />
          <span>Planr</span>
        </span>
      ),
      transparentMode: 'top',
    },
    links: [
      {
        text: 'Docs',
        url: '/docs',
        active: 'nested-url',
      },
      {
        text: 'Changelog',
        url: 'https://github.com/instructa/planr/blob/main/CHANGELOG.md',
        external: true,
      },
      {
        text: 'GitHub',
        url: 'https://github.com/instructa/planr',
        external: true,
      },
      {
        type: 'button',
        text: 'Get started',
        url: '/docs/getting-started/installation',
      },
    ],
  };
}
