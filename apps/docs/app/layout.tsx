import type { Metadata } from 'next';
import type { ReactNode } from 'react';
import { RootProvider } from 'fumadocs-ui/provider/next';
import { NoPrefetchLink } from '@/components/no-prefetch-link';
import './global.css';

export const metadata: Metadata = {
  metadataBase: new URL(process.env.NEXT_PUBLIC_SITE_URL ?? 'http://localhost:3000'),
  title: {
    default: 'Planr — Local-first coordination for coding agents',
    template: '%s · Planr',
  },
  description: 'Learn how to plan, coordinate, verify, and recover coding-agent work with Planr.',
  alternates: { canonical: '/' },
  applicationName: 'Planr',
  authors: [{ name: 'Planr contributors', url: 'https://github.com/instructa/planr' }],
  creator: 'Planr contributors',
  openGraph: {
    type: 'website',
    siteName: 'Planr',
    title: 'Planr — Local-first coordination for coding agents',
    description: 'Plan, coordinate, verify, and recover coding-agent work with durable local state.',
    images: [
      {
        url: '/og-image.png',
        width: 1200,
        height: 630,
        alt: 'Planr — Local-first coordination for coding agents',
      },
    ],
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Planr — Local-first coordination for coding agents',
    description: 'Plan, coordinate, verify, and recover coding-agent work with durable local state.',
    images: ['/og-image.png'],
  },
  formatDetection: { telephone: false },
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="flex min-h-screen flex-col">
        <RootProvider
          components={{ Link: NoPrefetchLink }}
          search={{ options: { type: 'static' } }}
        >
          {children}
        </RootProvider>
      </body>
    </html>
  );
}
