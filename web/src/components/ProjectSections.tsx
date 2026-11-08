import { useInfiniteQuery } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { api, queryKeys } from '../lib/api';
import type { SourceDescriptor } from '../lib/types';
import { Button } from './ui/button';
import { DecisionCard } from './DecisionCard';
import { SourceCard } from './SourceCard';

export function ProjectSections({ projectId, topic }: { projectId: string; topic: string | null }) {
  return (
    <div className="space-y-8">
      <DecisionsSection projectId={projectId} topic={topic} />
      <SourcesSection projectId={projectId} topic={topic} />
    </div>
  );
}

function DecisionsSection({ projectId, topic }: { projectId: string; topic: string | null }) {
  const query = useInfiniteQuery({
    queryKey: queryKeys.decisions(projectId, topic),
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam }) => api.decisions(projectId, pageParam, topic),
    getNextPageParam: (page) => page.next_before_id ?? undefined,
    refetchInterval: 5_000,
  });
  const decisions = [...new Map(query.data?.pages.flatMap((page) => page.decisions).map((decision) => [decision.id, decision]) ?? []).values()];

  return (
    <ProjectSection
      id="decisions"
      title="Decisions"
      hint={topic ? `Current accepted choices for “${topic}”.` : "Current accepted choices for this project. Superseded choices remain in history and are not shown here."}
      count={query.data ? `${decisions.length} current${query.hasNextPage ? ' loaded' : ''}` : null}
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
        <SectionEmpty message={topic ? `No current decisions for “${topic}”.` : "No current decisions for this project."} />
      ) : (
        <div className="space-y-2">
          {decisions.map((decision) => (
            <DecisionCard key={decision.id} decision={decision} />
          ))}
        </div>
      )}
      {query.hasNextPage ? (
        <Button variant="outline" size="sm" className="mt-3" onClick={() => { void query.fetchNextPage(); }} disabled={query.isFetching}>
          {query.isFetchingNextPage ? 'Loading…' : 'Load more decisions'}
        </Button>
      ) : null}
    </ProjectSection>
  );
}

function SourcesSection({ projectId, topic }: { projectId: string; topic: string | null }) {
  const query = useInfiniteQuery({
    queryKey: queryKeys.projectSources(projectId, topic),
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam }) => api.sources(projectId, pageParam, topic),
    getNextPageParam: (page) => page.next_before_id ?? undefined,
    refetchInterval: 5_000,
  });
  const sources = [...new Map(query.data?.pages.flatMap((page) => page.sources).map((source) => [source.id, source]) ?? []).values()];

  return (
    <ProjectSection
      id="sources"
      title="Sources"
      hint={topic ? `Sources for “${topic}”, including sources not linked to a task.` : "Every source in this project, including sources not linked to a task. Expand one to read inline Markdown or external metadata; locators are never fetched automatically."}
      count={query.data ? `${sources.length} ${query.hasNextPage ? 'loaded' : 'total'}` : null}
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
        <SectionEmpty message={topic ? `No sources for “${topic}”.` : "No sources in this project."} />
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
              topics={source.topics}
            />
          ))}
        </div>
      )}
      {query.hasNextPage ? (
        <Button variant="outline" size="sm" className="mt-3" onClick={() => { void query.fetchNextPage(); }} disabled={query.isFetching}>
          {query.isFetchingNextPage ? 'Loading…' : 'Load more sources'}
        </Button>
      ) : null}
    </ProjectSection>
  );
}

function sourceMeta(source: SourceDescriptor): string {
  return [
    source.representation,
    source.context_policy === 'always' ? 'always included' : 'when relevant',
    source.has_action ? 'has action' : null,
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
