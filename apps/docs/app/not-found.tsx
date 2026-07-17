import { FileQuestion } from 'lucide-react';
import Link from 'next/link';

export default function NotFound() {
  return (
    <main className="error-shell">
      <section className="error-card" aria-labelledby="not-found-title">
        <FileQuestion aria-hidden="true" />
        <p className="error-code">404 · Route not found</p>
        <h1 id="not-found-title">This page left the map.</h1>
        <p>
          The address may have changed, or the page may not exist yet. Search the documentation or
          return to a known route.
        </p>
        <div className="error-actions">
          <Link className="button-primary" href="/docs">Browse documentation</Link>
          <Link className="button-secondary" href="/">Return home</Link>
        </div>
      </section>
    </main>
  );
}
