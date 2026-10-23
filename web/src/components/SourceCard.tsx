import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { api, queryKeys } from '../lib/api';
import { formatDate } from '../lib/format';
import type { Source } from '../lib/types';
import { Markdown } from '../lib/markdown';
import { Button } from './ui/button';
import { Card } from './ui/card';
import { Meta } from './ui/meta';

export function SourceCard({
  projectId,
  sourceId,
  label,
  description,
  meta,
}: {
  projectId: string;
  sourceId: number;
  label?: string;
  description?: string;
  meta?: string;
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
          <p className="text-sm font-medium">
            {label ?? `Source #${sourceId}`}
          </p>
          <p className="text-xs text-muted-foreground">
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
      {source.action || source.snapshot || source.latest_attempt ? (
        <SourceState source={source} />
      ) : null}
    </div>
  );
}

function SourceState({ source }: { source: Source }) {
  const action = source.action;
  return (
    <dl className="grid gap-3 text-sm sm:grid-cols-2">
      {action ? (
        <Meta label="Action instruction">
          <Markdown text={action.instruction} />
        </Meta>
      ) : null}
      {action ? (
        <Meta label="Action trigger">
          {action.trigger.kind === 'manual'
            ? 'Manual'
            : `Every ${action.trigger.every_minutes} minutes`}
        </Meta>
      ) : null}
      <Meta label="Action due">
        {source.action_due ? 'Due now' : 'Not due'}
      </Meta>
      <Meta label="Next action">{formatDate(source.next_action_at)}</Meta>
      {source.snapshot ? (
        <div className="sm:col-span-2">
          <dt className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
            Latest successful snapshot
          </dt>
          <dd className="mt-0.5">
            <Markdown text={source.snapshot.summary} />
            <p className="mt-1 text-xs text-muted-foreground">
              Observed {formatDate(source.snapshot.observed_at)}
              {source.snapshot.checkpoint
                ? ` · checkpoint ${source.snapshot.checkpoint}`
                : ''}
            </p>
            {source.snapshot.references.length > 0 ? (
              <ul className="mt-1 space-y-1">
                {source.snapshot.references.map((reference) => (
                  <li key={reference}>
                    <code className="break-all text-xs">{reference}</code>
                  </li>
                ))}
              </ul>
            ) : null}
          </dd>
        </div>
      ) : null}
      {source.latest_attempt ? (
        <Meta label="Latest attempt">
          <span className="capitalize">{source.latest_attempt.outcome}</span>{' '}
          · {formatDate(source.latest_attempt.recorded_at)}
          {source.latest_attempt.error ? (
            <span className="mt-1 block text-xs text-destructive">
              {source.latest_attempt.error}
            </span>
          ) : null}
        </Meta>
      ) : null}
    </dl>
  );
}
