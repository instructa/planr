'use client';

import { RotateCcw } from 'lucide-react';
import Link from 'next/link';

export default function ErrorPage({ reset }: { reset: () => void }) {
  return (
    <main className="error-shell">
      <section className="error-card" aria-labelledby="error-title">
        <RotateCcw aria-hidden="true" />
        <p className="error-code">Rendering interrupted</p>
        <h1 id="error-title">The page could not finish loading.</h1>
        <p>Retry the route. If it still fails, return to the documentation index and report the URL.</p>
        <div className="error-actions">
          <button className="button-primary" type="button" onClick={reset}>Try again</button>
          <Link className="button-secondary" href="/docs">Browse documentation</Link>
        </div>
      </section>
    </main>
  );
}
