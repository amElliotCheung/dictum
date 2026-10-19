import { useQuery } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { api, queryKeys } from '../lib/api';
import { formatDate } from '../lib/format';
import type { SourceDescriptor } from '../lib/types';
import { Button } from './ui/button';
import { DecisionCard } from './DecisionCard';
import { SourceCard } from './SourceCard';

export function ProjectSections({ projectId }: { projectId: string }) {
  return (
    <div className="space-y-8">
      <DecisionsSection projectId={projectId} />
      <SourcesSection projectId={projectId} />
    </div>
  );
}

function DecisionsSection({ projectId }: { projectId: string }) {
  const query = useQuery({
    queryKey: queryKeys.decisions(projectId),
    queryFn: () => api.decisions(projectId),
    refetchInterval: 5_000,
  });
  const decisions = query.data?.decisions ?? [];

  return (
    <ProjectSection
      id="decisions"
      title="Decisions"
      hint="Current accepted choices for this project. Superseded choices remain in history and are not shown here."
      count={query.data ? `${decisions.length} current` : null}
    >
      {query.isError && query.data ? (
        <StaleNotice
          message={query.error.message}
          label="decisions"
          fetching={query.isFetching}
          onRetry={() => {
            void query.refetch();
          }}
        />
      ) : null}
      {query.isPending ? (
        <SectionLoading label="Loading decisions…" />
      ) : query.isError && !query.data ? (
        <SectionError
          title="Unable to load decisions"
          message={query.error.message}
          onRetry={() => {
            void query.refetch();
          }}
        />
      ) : decisions.length === 0 ? (
        <SectionEmpty message="No current decisions for this project." />
      ) : (
        <div className="space-y-2">
          {decisions.map((decision) => (
            <DecisionCard key={decision.id} decision={decision} />
          ))}
        </div>
      )}
    </ProjectSection>
  );
}

function SourcesSection({ projectId }: { projectId: string }) {
  const query = useQuery({
    queryKey: queryKeys.projectSources(projectId),
    queryFn: () => api.sources(projectId),
    refetchInterval: 5_000,
  });
  const sources = query.data?.sources ?? [];

  return (
    <ProjectSection
      id="sources"
      title="Sources"
      hint="Every source in this project, including sources not linked to a task. Expand one to read inline Markdown or external metadata; locators are never fetched automatically."
      count={query.data ? `${sources.length} total` : null}
    >
      {query.isError && query.data ? (
        <StaleNotice
          message={query.error.message}
          label="sources"
          fetching={query.isFetching}
          onRetry={() => {
            void query.refetch();
          }}
        />
      ) : null}
      {query.isPending ? (
        <SectionLoading label="Loading sources…" />
      ) : query.isError && !query.data ? (
        <SectionError
          title="Unable to load sources"
          message={query.error.message}
          onRetry={() => {
            void query.refetch();
          }}
        />
      ) : sources.length === 0 ? (
        <SectionEmpty message="No sources in this project." />
      ) : (
        <div className="grid gap-2 lg:grid-cols-2">
          {sources.map((source) => (
            <SourceCard
              key={`${projectId}:${source.id}`}
              projectId={projectId}
              sourceId={source.id}
              label={`${source.name} · #${source.id}`}
              description={source.description}
              meta={sourceMeta(source)}
            />
          ))}
        </div>
      )}
    </ProjectSection>
  );
}

function sourceMeta(source: SourceDescriptor): string {
  return [
    source.representation,
    source.context_policy === 'always' ? 'always included' : 'when relevant',
    source.has_action
      ? source.action_due
        ? 'action due'
        : source.next_action_at
          ? `next action ${formatDate(source.next_action_at)}`
          : 'manual action'
      : null,
  ]
    .filter((value): value is string => value !== null)
    .join(' · ');
}

function ProjectSection({
  id,
  title,
  hint,
  count,
  children,
}: {
  id: string;
  title: string;
  hint: string;
  count: string | null;
  children: ReactNode;
}) {
  return (
    <section aria-labelledby={`${id}-heading`} className="border-t pt-6">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 id={`${id}-heading`} className="text-base font-semibold">
          {title}
        </h2>
        {count ? (
          <span className="text-xs text-muted-foreground">{count}</span>
        ) : null}
      </div>
      <p className="mb-3 mt-1 max-w-4xl text-xs text-muted-foreground">
        {hint}
      </p>
      {children}
    </section>
  );
}

function StaleNotice({
  message,
  label,
  fetching,
  onRetry,
}: {
  message: string;
  label: string;
  fetching: boolean;
  onRetry: () => void;
}) {
  return (
    <div
      role="status"
      className="mb-3 flex flex-wrap items-center gap-2 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900"
    >
      <span>
        Refresh failed: {message}. Showing the last successfully loaded{' '}
        {label}.
      </span>
      <Button
        variant="outline"
        size="sm"
        onClick={onRetry}
        disabled={fetching}
      >
        {fetching ? 'Retrying…' : 'Retry'}
      </Button>
    </div>
  );
}

function SectionLoading({ label }: { label: string }) {
  return (
    <div
      role="status"
      className="rounded-lg border border-dashed bg-card p-6 text-center text-sm text-muted-foreground"
    >
      {label}
    </div>
  );
}

function SectionEmpty({ message }: { message: string }) {
  return (
    <p className="rounded-lg border border-dashed bg-card p-6 text-center text-sm text-muted-foreground">
      {message}
    </p>
  );
}

function SectionError({
  title,
  message,
  onRetry,
}: {
  title: string;
  message: string;
  onRetry: () => void;
}) {
  return (
    <div
      role="alert"
      className="rounded-lg border border-rose-200 bg-rose-50 p-4 text-rose-900"
    >
      <p className="text-sm font-semibold">{title}</p>
      <p className="mt-1 text-sm">{message}</p>
      <Button variant="outline" size="sm" className="mt-2" onClick={onRetry}>
        Retry
      </Button>
    </div>
  );
}
