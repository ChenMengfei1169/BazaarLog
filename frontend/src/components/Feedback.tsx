// Reusable UI primitives: a top-level error banner and a loading spinner.
// Kept minimal so components can compose them inline without prop drilling.
import type { ReactNode } from 'react';

export function ErrorBanner({ message }: { message: string }): JSX.Element {
  return (
    <div className="card border-negative text-negative text-sm" role="alert">
      {message}
    </div>
  );
}

export function Loading(): JSX.Element {
  return (
    <div className="text-ink-300 text-sm animate-pulse" aria-live="polite">
      Loading...
    </div>
  );
}

export function PageShell({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}): JSX.Element {
  return (
    <section className="space-y-4">
      <h2 className="text-2xl font-semibold tracking-tight">{title}</h2>
      {children}
    </section>
  );
}
