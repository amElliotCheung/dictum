import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { api, queryKeys } from '../lib/api';
import type { Source, Topic } from '../lib/types';
import { Markdown } from '../lib/markdown';
import { Button } from './ui/button';
import { Card } from './ui/card';
import { Badge } from './ui/badge';
import { Meta } from './ui/meta';

export function SourceCard({
  projectId,
  sourceId,
  label,
  description,
  meta,
  topics,
}: {
  projectId: string;
  sourceId: string;
  label?: string;
  description?: string;
  meta?: string;
  topics?: Topic[];
}) {
  const [open, setOpen] = useState(false);
  const sourceQuery = useQuery({
    queryKey: queryKeys.source(projectId, sourceId),
    queryFn: () => api.source(projectId, sourceId),
    enabled: open,
    refetchInterval: open ? 5_000 : false,
  });

  return (
    <Card className="p-3">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="break-words text-sm font-medium">
            {label ?? `Source #${sourceId}`}
          </p>
          {topics ? <div className="my-1 flex flex-wrap gap-1" aria-label="Topics">
            {topics.map((topic) => <Badge key={topic.id} variant="secondary">{topic.name}</Badge>)}
          </div> : null}
          <p className="break-words text-xs text-muted-foreground">
            {description ?? `Source #${sourceId}`}
            {meta ? ` · ${meta}` : ''}
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => setOpen((value) => !value)}
          aria-expanded={open}
        >
          {open ? 'Hide source' : 'Expand source'}
        </Button>
      </div>
      {open ? (
        <div className="mt-3 border-t pt-3">
          {sourceQuery.isError ? (
            <div
              role={sourceQuery.data ? 'status' : 'alert'}
              className="mb-3 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900"
            >
              <p>
                {sourceQuery.data
                  ? 'Source refresh failed: '
                  : 'Unable to load source: '}
                {sourceQuery.error.message}.
                {sourceQuery.data
                  ? ' Showing the last successfully loaded source.'
                  : ''}
              </p>
              <Button
                variant="outline"
                size="sm"
                className="mt-2"
                onClick={() => {
                  void sourceQuery.refetch();
                }}
                disabled={sourceQuery.isFetching}
              >
                {sourceQuery.isFetching ? 'Retrying…' : 'Retry source'}
              </Button>
            </div>
          ) : null}
          {sourceQuery.isPending ? (
            <p className="text-sm text-muted-foreground" role="status">
              Loading source…
            </p>
          ) : sourceQuery.data ? (
            <SourceBody source={sourceQuery.data} />
          ) : null}
        </div>
      ) : null}
    </Card>
  );
}

export function SourceBody({ source }: { source: Source }) {
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap gap-1" aria-label="Topics">
        {source.topics.map((topic) => <Badge key={topic.id} variant="secondary">{topic.name}</Badge>)}
      </div>
      <p className="text-sm text-muted-foreground">{source.description}</p>
      {source.content !== null ? (
        <Markdown text={source.content} />
      ) : (
        <div className="space-y-1">
          <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
            Locator (not auto-fetched)
          </p>
          <code className="block break-all rounded bg-muted px-2 py-1 text-xs">
            {source.locator ?? '—'}
          </code>
        </div>
      )}
      {source.action ? (
        <dl className="text-sm">
          <Meta label="Action instruction">
            <Markdown text={source.action.instruction} />
          </Meta>
        </dl>
      ) : null}
    </div>
  );
}
