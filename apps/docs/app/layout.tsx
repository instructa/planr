import type { Metadata } from 'next';
import type { ReactNode } from 'react';
import { RootProvider } from 'fumadocs-ui/provider/next';
import './global.css';

export const metadata: Metadata = {
  metadataBase: new URL(process.env.NEXT_PUBLIC_SITE_URL ?? 'http://localhost:3000'),
  title: {
    default: 'Planr Documentation',
    template: '%s · Planr',
  },
  description: 'Learn how to plan, coordinate, verify, and recover coding-agent work with Planr.',
  applicationName: 'Planr Documentation',
  authors: [{ name: 'Planr contributors', url: 'https://github.com/instructa/planr' }],
  creator: 'Planr contributors',
  openGraph: {
    type: 'website',
    siteName: 'Planr Documentation',
    title: 'Planr Documentation',
    description: 'Plan, coordinate, verify, and recover coding-agent work with durable local state.',
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Planr Documentation',
    description: 'Plan, coordinate, verify, and recover coding-agent work with durable local state.',
  },
  formatDetection: { telephone: false },
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="flex min-h-screen flex-col">
        <RootProvider>{children}</RootProvider>
      </body>
    </html>
  );
}
