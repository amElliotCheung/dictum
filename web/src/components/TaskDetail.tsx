import {
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { X } from 'lucide-react';
import { api, ApiError, queryKeys } from '../lib/api';
import {
  formatDate,
  formatKind,
  statusLabels,
} from '../lib/format';
import type {
  Decision,
  Source,
  TaskContext,
  TaskResult,
  TaskResultList,
  TaskStatus,
  TaskSummary,
} from '../lib/types';
import { Markdown } from '../lib/markdown';
import { Badge } from './ui/badge';
import { Button } from './ui/button';
import { Card } from './ui/card';

interface HistoryState {
  requested: boolean;
  pending: boolean;
  fetchingOlder: boolean;
  hasNext: boolean;
  error: string | null;
  onLoadOlder: () => void;
  onRetry: () => void;
}

export function TaskDetail({
  projectId,
  taskId,
  tasks,
  onClose,
  onOpenTask,
}: {
  projectId: string;
  taskId: number;
  tasks: TaskSummary[];
  onClose: () => void;
  onOpenTask: (taskId: number) => void;
}) {
  const closeRef = useRef<HTMLButtonElement>(null);
  const [historyRequested, setHistoryRequested] = useState(false);
  const validTask = Number.isInteger(taskId) && taskId > 0;

  useEffect(() => {
    setHistoryRequested(false);
  }, [taskId]);

  useEffect(() => {
    closeRef.current?.focus();
  }, [taskId]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [onClose]);

  const contextQuery = useQuery({
    queryKey: queryKeys.context(projectId, taskId),
    queryFn: () => api.context(projectId, taskId),
    enabled: validTask,
    refetchInterval: 5000,
  });

  const resultsQuery = useInfiniteQuery({
    queryKey: queryKeys.results(projectId, taskId),
    queryFn: ({ pageParam }) => api.results(projectId, taskId, pageParam),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (lastPage: TaskResultList) =>
      lastPage.next_before_id ?? undefined,
    enabled: validTask && historyRequested,
    refetchInterval: historyRequested ? 5000 : false,
  });

  const context = contextQuery.data;
  const contextError = contextQuery.error;
  const notFound =
    contextError instanceof ApiError && contextError.status === 404;
  const boundedResults: TaskResult[] = context
    ? [context.latest_result, context.latest_successful_result].filter(
        (result): result is TaskResult => result !== null,
      )
    : [];
  const history = mergeResults(boundedResults, resultsQuery.data?.pages);

  const historyState: HistoryState = {
    requested: historyRequested,
    pending: resultsQuery.isPending,
    fetchingOlder: resultsQuery.isFetchingNextPage,
    hasNext: resultsQuery.hasNextPage,
    error: resultsQuery.isError
      ? (resultsQuery.error.message ?? 'Unknown error')
      : null,
    onLoadOlder: () => {
      if (!historyRequested) {
        setHistoryRequested(true);
      } else if (resultsQuery.hasNextPage && !resultsQuery.isFetchingNextPage) {
        void resultsQuery.fetchNextPage();
      }
    },
    onRetry: () => {
      void resultsQuery.refetch();
    },
  };

  return (
    <>
      <div
        aria-hidden="true"
        onClick={onClose}
        className="fixed inset-0 z-20 bg-slate-950/20 backdrop-blur-[1px]"
      />
      <aside
        role="dialog"
        aria-modal="false"
        aria-labelledby="task-detail-title"
        tabIndex={-1}
        className="fixed inset-y-0 right-0 z-30 flex w-full max-w-2xl flex-col border-l bg-card shadow-2xl"
      >
        <div className="flex items-start justify-between gap-3 border-b px-4 py-3">
          <div className="min-w-0">
            <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              Task {validTask ? `#${taskId}` : ''}
            </p>
            <h2
              id="task-detail-title"
              className="truncate text-lg font-semibold"
            >
              {context?.task.title ??
                (validTask ? 'Loading task…' : 'Invalid task selection')}
            </h2>
          </div>
          <Button
            ref={closeRef}
            variant="ghost"
            size="icon"
            onClick={onClose}
            aria-label="Close task details"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        <div className="flex-1 overflow-y-auto px-4 py-4">
          {!validTask ? (
            <PanelState
              title="Invalid task selection"
              message="The task ID in this URL is not a positive integer. Close this panel and choose a task from the board."
            />
          ) : contextQuery.isPending ? (
            <LoadingDetail />
          ) : contextQuery.isError && !context ? (
            <PanelState
              title={notFound ? 'Task not found' : 'Unable to load task'}
              message={
                notFound
                  ? `No task #${taskId} exists in this project. It may belong to another project or have been removed.`
                  : (contextError?.message ?? 'Unknown error')
              }
              onRetry={() => {
                void contextQuery.refetch();
              }}
            />
          ) : context ? (
            <>
              {contextQuery.isError ? (
                <div
                  role="status"
                  className="mb-4 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900"
                >
                  Refresh failed: {contextQuery.error.message}. Showing the last
                  successfully loaded task.
                </div>
              ) : null}
              <DetailBody
                projectId={projectId}
                context={context}
                tasks={tasks}
                history={history}
                historyState={historyState}
                onOpenTask={onOpenTask}
              />
            </>
          ) : null}
        </div>
      </aside>
    </>
  );
}

function DetailBody({
  projectId,
  context,
  tasks,
  history,
  historyState,
  onOpenTask,
}: {
  projectId: string;
  context: TaskContext;
  tasks: TaskSummary[];
  history: TaskResult[];
  historyState: HistoryState;
  onOpenTask: (taskId: number) => void;
}) {
  const { task, dependency_state } = context;

  return (
    <div className="space-y-6">
      <dl className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Meta label="Kind">{formatKind(task.kind)}</Meta>
        <Meta label="Status">
          <StatusBadge status={task.status} />
        </Meta>
        <Meta label="Revision">{task.revision}</Meta>
        <Meta label="Trigger">{task.trigger_event ?? '—'}</Meta>
        <Meta label="Created">{formatDate(task.created_at)}</Meta>
        <Meta label="Updated">{formatDate(task.updated_at)}</Meta>
      </dl>

      {task.kind === 'recurring' ? (
        <p className="rounded-lg border border-violet-200 bg-violet-50 px-3 py-2 text-xs text-violet-900">
          Active is eligibility for externally authorized work. It does not mean
          an agent is running or that a schedule is installed.
        </p>
      ) : null}

      {task.progress ? (
        <Section title="Progress">
          <Markdown text={task.progress} />
        </Section>
      ) : null}

      <Section title="Brief">
        <div className="space-y-4">
          <div>
            <Subheading>Problem or goal</Subheading>
            <Markdown text={task.problem_or_goal} />
          </div>
          <div>
            <Subheading>Design</Subheading>
            <Markdown text={task.design} />
          </div>
          <div>
            <Subheading>Acceptance criteria</Subheading>
            <ol className="list-decimal space-y-2 pl-5">
              {task.acceptance_criteria.map((criterion, index) => (
                <li key={index}>
                  <Markdown text={criterion} />
                </li>
              ))}
            </ol>
          </div>
          {task.alternatives_rejected ? (
            <div>
              <Subheading>Alternatives rejected</Subheading>
              <Markdown text={task.alternatives_rejected} />
            </div>
          ) : null}
        </div>
      </Section>

      <Section title="Dependencies">
        <div className="grid gap-4 sm:grid-cols-2">
          <DependencyGroup
            title="Before start"
            ids={task.prerequisite_task_ids}
            unmet={dependency_state.unmet_prerequisite_task_ids}
            tasks={tasks}
            onOpenTask={onOpenTask}
          />
          <DependencyGroup
            title={task.kind === 'recurring' ? 'Before success' : 'Before done'}
            ids={task.completion_task_ids}
            unmet={dependency_state.unmet_completion_task_ids}
            tasks={tasks}
            onOpenTask={onOpenTask}
          />
        </div>
        <p className="mt-3 text-xs text-muted-foreground">
          {task.kind === 'recurring'
            ? 'Recurring tasks never reach done. All dependencies must be done before recording success or no change; failure reports remain allowed.'
            : dependency_state.ready_to_start
              ? 'Ready to start: all before-start gates are done. Completion gates still apply before done.'
              : 'Lifecycle status and dependency readiness are separate. A todo task can still be blocked by an unmet before-start gate.'}
        </p>
      </Section>

      <Section title="Current decisions">
        {context.current_decisions.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No current decisions for this project.
          </p>
        ) : (
          <div className="space-y-2">
            {context.current_decisions.map((decision) => (
              <DecisionCard key={decision.id} decision={decision} />
            ))}
          </div>
        )}
      </Section>

      <Section title="Context sources">
        <p className="mb-3 text-xs text-muted-foreground">
          Inline documents are returned in full. External sources show their
          locator, action and state only; the dashboard never fetches a locator
          automatically.
        </p>
        {context.sources.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No context sources for this task.
          </p>
        ) : (
          <div className="space-y-2">
            {context.sources.map((source) => (
              <details
                key={source.id}
                className="rounded-lg border bg-card"
              >
                <summary className="cursor-pointer px-3 py-2 text-sm font-medium">
                  {source.name}
                  <span className="ml-2 text-xs font-normal text-muted-foreground">
                    {source.context_policy === 'always'
                      ? 'always included'
                      : 'when relevant'}{' '}
                    · {source.content !== null ? 'inline' : 'external'}
                  </span>
                </summary>
                <div className="border-t px-3 py-3">
                  <SourceBody source={source} />
                </div>
              </details>
            ))}
          </div>
        )}
      </Section>

      <Section
        title="Execution history"
        aside={
          <span className="text-xs text-muted-foreground">
            {history.length} loaded
          </span>
        }
      >
        <p className="mb-3 text-xs text-muted-foreground">
          Starts with the bounded latest and latest-successful entries. Older
          pages use the same keyset pagination as MCP. Evidence is
          agent-reported; recorded_at is server receipt time. Recording a result
          never changes lifecycle.
        </p>
        {history.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No execution history recorded.
          </p>
        ) : (
          <div className="space-y-2">
            {history.map((result) => (
              <ResultCard
                key={result.id}
                result={result}
                projectId={projectId}
              />
            ))}
          </div>
        )}
        {historyState.error ? (
          <div
            role="alert"
            className="mt-3 flex flex-wrap items-center gap-2 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs text-rose-900"
          >
            <span>Could not load older results: {historyState.error}</span>
            <Button
              variant="outline"
              size="sm"
              onClick={historyState.onRetry}
            >
              Retry
            </Button>
          </div>
        ) : null}
        <div className="mt-3">
          <Button
            variant="outline"
            size="sm"
            onClick={historyState.onLoadOlder}
            disabled={
              historyState.requested &&
              (historyState.pending ||
                historyState.fetchingOlder ||
                !historyState.hasNext)
            }
          >
            {historyState.requested
              ? historyState.pending || historyState.fetchingOlder
                ? 'Loading…'
                : historyState.hasNext
                  ? 'Load older results'
                  : 'End of recorded history'
              : 'Load older results'}
          </Button>
        </div>
      </Section>
    </div>
  );
}

function Section({
  title,
  aside,
  children,
}: {
  title: string;
  aside?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="border-t pt-5">
      <div className="mb-2 flex items-center justify-between gap-2">
        <h3 className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
          {title}
        </h3>
        {aside}
      </div>
      {children}
    </section>
  );
}

function Subheading({ children }: { children: ReactNode }) {
  return (
    <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
      {children}
    </h4>
  );
}

function Meta({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <dt className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </dt>
      <dd className="mt-0.5 text-sm">{children}</dd>
    </div>
  );
}

const statusClasses: Record<TaskStatus, string> = {
  todo: 'border-sky-300 bg-sky-50 text-sky-800',
  in_progress: 'border-amber-300 bg-amber-50 text-amber-800',
  blocked: 'border-rose-300 bg-rose-50 text-rose-800',
  done: 'border-emerald-300 bg-emerald-50 text-emerald-800',
  active: 'border-violet-300 bg-violet-50 text-violet-800',
  paused: 'border-slate-300 bg-slate-50 text-slate-700',
};

function StatusBadge({ status }: { status: TaskStatus }) {
  return (
    <Badge variant="outline" className={statusClasses[status]}>
      {statusLabels[status]}
    </Badge>
  );
}

function DependencyGroup({
  title,
  ids,
  unmet,
  tasks,
  onOpenTask,
}: {
  title: string;
  ids: number[];
  unmet: number[];
  tasks: TaskSummary[];
  onOpenTask: (taskId: number) => void;
}) {
  return (
    <div>
      <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </h4>
      {ids.length === 0 ? (
        <p className="mt-1 text-sm text-muted-foreground">None</p>
      ) : (
        <div className="mt-1 flex flex-wrap gap-2">
          {ids.map((id) => {
            const blocked = unmet.includes(id);
            const task = tasks.find((candidate) => candidate.id === id);
            return (
              <Button
                key={id}
                variant="outline"
                size="sm"
                className={
                  blocked
                    ? 'border-rose-300 bg-rose-50 text-rose-800'
                    : undefined
                }
                onClick={() => onOpenTask(id)}
                title={task?.title}
              >
                #{id}
                {blocked ? ' · unmet' : ''}
              </Button>
            );
          })}
        </div>
      )}
    </div>
  );
}

function DecisionCard({ decision }: { decision: Decision }) {
  return (
    <Card className="p-3">
      <Markdown text={decision.statement} />
      <p className="mt-2 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        Rationale
      </p>
      <Markdown text={decision.rationale} />
      <p className="mt-2 text-[11px] text-muted-foreground">
        Decision #{decision.id} · {formatDate(decision.created_at)}
      </p>
    </Card>
  );
}

function SourceBody({ source }: { source: Source }) {
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

function ExpandableSource({
  projectId,
  sourceId,
  label,
  description,
  representation,
}: {
  projectId: string;
  sourceId: number;
  label?: string;
  description?: string;
  representation?: string;
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
            {representation ? ` · ${representation}` : ''}
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

function ResultCard({
  result,
  projectId,
}: {
  result: TaskResult;
  projectId: string;
}) {
  const outcomeClass =
    result.outcome === 'success'
      ? 'border-emerald-300 bg-emerald-50 text-emerald-800'
      : result.outcome === 'no_change'
        ? 'border-sky-300 bg-sky-50 text-sky-800'
        : 'border-rose-300 bg-rose-50 text-rose-800';
  const outcomeLabel =
    result.outcome === 'no_change' ? 'No change' : result.outcome;
  return (
    <Card className="p-3">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="outline" className={outcomeClass}>
          {outcomeLabel}
        </Badge>
        <span className="text-xs text-muted-foreground">
          #{result.id} · recorded {formatDate(result.recorded_at)} · task
          revision {result.task_revision}
        </span>
      </div>
      <div className="mt-2">
        <Markdown text={result.evidence} />
      </div>
      {result.output_source_ids.length > 0 ? (
        <div className="mt-3 space-y-2">
          {result.output_source_ids.map((sourceId) => (
            <ExpandableSource
              key={sourceId}
              projectId={projectId}
              sourceId={sourceId}
              label={`Output source #${sourceId}`}
            />
          ))}
        </div>
      ) : null}
      <p className="mt-2 text-[11px] text-muted-foreground">
        Agent-reported evidence; recorded_at is the server receipt time, not
        independently observed execution time.
      </p>
    </Card>
  );
}

function LoadingDetail() {
  return (
    <div className="space-y-3" role="status" aria-label="Loading task details">
      {Array.from({ length: 6 }).map((_, index) => (
        <div
          key={index}
          className="h-16 animate-pulse rounded-lg bg-muted"
        />
      ))}
    </div>
  );
}

function PanelState({
  title,
  message,
  onRetry,
}: {
  title: string;
  message: string;
  onRetry?: () => void;
}) {
  return (
    <div className="rounded-xl border border-dashed bg-muted/40 p-6">
      <h3 className="text-sm font-semibold">{title}</h3>
      <p className="mt-1 text-sm text-muted-foreground">{message}</p>
      {onRetry ? (
        <Button
          variant="outline"
          size="sm"
          className="mt-3"
          onClick={onRetry}
        >
          Retry
        </Button>
      ) : null}
    </div>
  );
}

function mergeResults(
  bounded: TaskResult[],
  pages: TaskResultList[] | undefined,
): TaskResult[] {
  const byId = new Map<number, TaskResult>();
  for (const result of bounded) {
    byId.set(result.id, result);
  }
  for (const page of pages ?? []) {
    for (const result of page.results) {
      if (!byId.has(result.id)) {
        byId.set(result.id, result);
      }
    }
  }
  return [...byId.values()].sort((left, right) => right.id - left.id);
}
