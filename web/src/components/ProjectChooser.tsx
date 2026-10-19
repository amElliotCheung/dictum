import { Button } from './ui/button';
import { formatDate } from '../lib/format';
import type { Project } from '../lib/types';

export function ProjectChooser({
  projects,
  error,
  onSelect,
  onRetry,
}: {
  projects: Project[];
  error: string | null;
  onSelect: (projectId: string) => void;
  onRetry: () => void;
}) {
  return (
    <section aria-labelledby="chooser-heading" className="mx-auto max-w-5xl">
      <h1
        id="chooser-heading"
        className="text-2xl font-semibold tracking-tight"
      >
        Choose a project
      </h1>
      <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
        Every project has its own tasks, sources and decisions. Names can
        repeat, so each project shows its full distinguishing ID.
      </p>

      {error ? (
        <div
          role="alert"
          className="mt-4 flex flex-wrap items-center gap-3 rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-900"
        >
          <span>Could not refresh projects: {error}</span>
          <Button variant="outline" size="sm" onClick={onRetry}>
            Retry
          </Button>
        </div>
      ) : null}

      {projects.length === 0 ? (
        <p className="mt-6 rounded-xl border border-dashed bg-card p-8 text-center text-sm text-muted-foreground">
          No projects yet. Create one through the Dictum MCP server.
        </p>
      ) : (
        <ul className="mt-6 grid gap-3 sm:grid-cols-2">
          {projects.map((project) => (
            <li key={project.id}>
              <button
                type="button"
                onClick={() => onSelect(project.id)}
                className="flex h-full w-full flex-col rounded-xl border bg-card p-4 text-left shadow-sm transition hover:border-primary/40 hover:shadow-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="text-base font-semibold">{project.name}</span>
                <span className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
                  {project.id}
                </span>
                <span className="mt-3 text-sm text-muted-foreground">
                  {project.description}
                </span>
                <span className="mt-3 text-xs text-muted-foreground">
                  Created {formatDate(project.created_at)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
