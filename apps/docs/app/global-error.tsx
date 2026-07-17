'use client';

import Link from 'next/link';

export default function GlobalError({ reset }: { reset: () => void }) {
  return (
    <html lang="en">
      <body>
        <main className="error-shell">
          <section className="error-card" aria-labelledby="global-error-title">
            <p className="error-code">Application error</p>
            <h1 id="global-error-title">The documentation shell could not load.</h1>
            <p>Retry once. If the problem persists, report it with the page address.</p>
            <div className="error-actions">
              <button className="button-primary" type="button" onClick={reset}>Try again</button>
              <Link className="button-secondary" href="/">Return home</Link>
            </div>
          </section>
        </main>
      </body>
    </html>
  );
}
