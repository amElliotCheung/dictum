import { useQuery } from '@tanstack/react-query';
import { api, queryKeys } from '../lib/api';
import { Markdown } from '../lib/markdown';
import { Button } from './ui/button';

export function PromotedDecisions({ projectId }: { projectId: string }) {
  const query = useQuery({
    queryKey: queryKeys.projectContext(projectId),
    queryFn: () => api.projectContext(projectId),
    refetchInterval: 5_000,
  });
  const decisions = query.data?.project.promoted_decisions ?? [];

  if (query.data && decisions.length === 0 && !query.isError) {
    return null;
  }

  return (
    <section aria-labelledby="promoted-decisions-heading" className="mt-4 max-w-4xl">
      <h2 id="promoted-decisions-heading" className="text-sm font-semibold">
        Promoted decisions
      </h2>
      {query.isError ? (
        <div
          role={query.data ? 'status' : 'alert'}
          className={`mt-2 rounded-lg border px-3 py-2 text-xs ${query.data ? 'border-amber-200 bg-amber-50 text-amber-900' : 'border-rose-200 bg-rose-50 text-rose-900'}`}
        >
          <p>
            {query.data ? 'Refresh failed' : 'Unable to load promoted decisions'}: {query.error.message}.
            {query.data ? ' Showing the last successfully loaded promoted decisions.' : ''}
          </p>
          <Button
            variant="outline"
            size="sm"
            className="mt-2"
            disabled={query.isFetching}
            onClick={() => { void query.refetch(); }}
          >
            {query.isFetching ? 'Retrying…' : 'Retry'}
          </Button>
        </div>
      ) : null}
      {query.isPending ? (
        <p role="status" className="mt-2 text-sm text-muted-foreground">
          Loading promoted decisions…
        </p>
      ) : decisions.length > 0 ? (
        <ol className="mt-2 space-y-2">
          {decisions.map((decision) => (
            <li
              key={decision.reference_id}
              data-decision-id={decision.id}
              data-reference-id={decision.reference_id}
              className="rounded-lg border border-primary/20 bg-secondary/40 px-3 py-2"
            >
              <Markdown text={decision.statement} />
              <p className="mt-1 break-all text-[11px] text-muted-foreground">
                Decision #{decision.id}
              </p>
            </li>
          ))}
        </ol>
      ) : null}
    </section>
  );
}
