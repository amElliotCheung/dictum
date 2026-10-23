import { useEffect, useState, type ReactNode } from 'react';
import {
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query';
import { Play, Square } from 'lucide-react';
import { api, ApiError, queryKeys } from '../lib/api';
import { formatDate, formatIds } from '../lib/format';
import { cn } from '../lib/utils';
import type {
  DependencyState,
  RunAgent,
  RunDetail,
  RunStatus,
  RunSummary,
  Task,
} from '../lib/types';
import { Markdown } from '../lib/markdown';
import { Badge } from './ui/badge';
import { Button } from './ui/button';
import { Card } from './ui/card';
import { Input } from './ui/input';
import { Meta } from './ui/meta';

const runStatusLabels: Record<RunStatus, string> = {
  running: 'Running',
  succeeded: 'Process succeeded',
  failed: 'Stopped with failure',
  cancelled: 'Cancelled',
  timed_out: 'Timed out',
  interrupted: 'Interrupted',
};

const runStatusClasses: Record<RunStatus, string> = {
  running: 'border-amber-300 bg-amber-50 text-amber-800',
  succeeded: 'border-emerald-300 bg-emerald-50 text-emerald-800',
  failed: 'border-rose-300 bg-rose-50 text-rose-800',
  cancelled: 'border-slate-300 bg-slate-50 text-slate-700',
  timed_out: 'border-orange-300 bg-orange-50 text-orange-800',
  interrupted: 'border-zinc-300 bg-zinc-50 text-zinc-700',
};

export function RunPanel({
  projectId,
  task,
  dependencyState,
}: {
  projectId: string;
  task: Task;
  dependencyState: DependencyState;
}) {
  const queryClient = useQueryClient();
  const storageKey = `dictum.run.cwd.${projectId}`;
  const [agent, setAgent] = useState<RunAgent>(() =>
    storedAgent() ?? 'codex',
  );
  const [workingDirectory, setWorkingDirectory] = useState(
    () => storedValue(storageKey) ?? '',
  );
  const [selectedRunId, setSelectedRunId] = useState<number | null>(null);

  useEffect(() => {
    setWorkingDirectory(storedValue(storageKey) ?? '');
  }, [storageKey]);

  const configQuery = useQuery({
    queryKey: queryKeys.runConfig,
    queryFn: api.runConfig,
    staleTime: 60_000,
  });

  const runsQuery = useQuery({
    queryKey: queryKeys.runs(projectId, task.id),
    queryFn: () => api.runs(projectId, task.id),
    refetchInterval: (query) =>
      query.state.data?.runs.some((run) => run.status === 'running')
        ? 5_000
        : 15_000,
  });

  const runs = runsQuery.data?.runs ?? [];
  const selected = runs.find((run) => run.id === selectedRunId) ?? null;

  useEffect(() => {
    setSelectedRunId(null);
  }, [projectId, task.id]);

  useEffect(() => {
    // Runs are append-only, and a launch may select an ID before the refetched
    // list contains it. Only choose the newest run when nothing is selected.
    if (selectedRunId === null && runs.length > 0) {
      setSelectedRunId(runs[0].id);
    }
  }, [runs, selectedRunId]);

  const detailQuery = useQuery({
    queryKey: queryKeys.run(projectId, task.id, selectedRunId ?? 0),
    queryFn: () => api.run(projectId, task.id, selectedRunId ?? 0, false),
    enabled: selectedRunId !== null,
    // Follow this query's own snapshot, not the list. finish_run writes the
    // terminal status and final output in one update, so the snapshot that
    // observes the transition is already complete.
    refetchInterval: (query) =>
      query.state.data?.run.status === 'running' ? 1_000 : false,
  });

  const launch = useMutation({
    mutationFn: () => {
      if (!configQuery.data) {
        throw new Error('Run configuration is not available yet.');
      }
      return api.launchRun(
        projectId,
        task.id,
        configQuery.data.token,
        agent,
        workingDirectory.trim(),
      );
    },
    onSuccess: (run) => {
      setSelectedRunId(run.id);
      void queryClient.invalidateQueries({
        queryKey: queryKeys.runs(projectId, task.id),
      });
    },
    onError: (error) => {
      if (error instanceof ApiError && error.status === 403) {
        void configQuery.refetch();
      }
    },
  });

  const cancel = useMutation({
    mutationFn: (runId: number) => {
      if (!configQuery.data) {
        throw new Error('Run configuration is not available yet.');
      }
      return api.cancelRun(
        projectId,
        task.id,
        runId,
        configQuery.data.token,
      );
    },
    onSuccess: (run) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.runs(projectId, task.id),
      });
      void queryClient.invalidateQueries({
        queryKey: queryKeys.run(projectId, task.id, run.id),
      });
    },
    onError: (error) => {
      if (error instanceof ApiError && error.status === 403) {
        void configQuery.refetch();
      }
    },
  });

  const blockedReason = runBlockedReason(task, dependencyState);
  const activeRun = runs.find((run) => run.status === 'running') ?? null;
  const error =
    launch.error instanceof Error
      ? launch.error.message
      : cancel.error instanceof Error
        ? cancel.error.message
        : null;

  return (
    <section
      aria-labelledby="run-heading"
      className="rounded-xl border bg-muted/30 p-4"
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div>
          <h3 id="run-heading" className="text-sm font-semibold">
            Run once
          </h3>
          <p className="mt-1 max-w-xl text-xs text-muted-foreground">
            One deliberate action starts one bounded local agent attempt. Its
            Dictum connection is read-only except for recording one task result,
            so it cannot edit the task, change status or record review feedback.
            Process exit alone never completes the task.
          </p>
        </div>
        <Badge variant="outline">
          {configQuery.data
            ? `Limit ${formatLimit(configQuery.data.timeout_seconds)}`
            : 'Loading limit…'}
        </Badge>
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-[10rem_1fr]">
        <label className="text-xs font-medium">
          Agent
          <select
            value={agent}
            onChange={(event) => {
              const value = event.target.value as RunAgent;
              setAgent(value);
              storeValue('dictum.run.agent', value);
            }}
            className="mt-1 h-9 w-full rounded-md border border-input bg-card px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <option value="codex">Codex</option>
            <option value="claude">Claude Code</option>
          </select>
        </label>
        <label className="text-xs font-medium">
          Working directory (absolute)
          <Input
            value={workingDirectory}
            onChange={(event) => {
              setWorkingDirectory(event.target.value);
              storeValue(storageKey, event.target.value);
            }}
            placeholder="/absolute/path/to/repository"
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            className="mt-1 font-mono text-xs"
          />
        </label>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <Button
          onClick={() => launch.mutate()}
          disabled={
            launch.isPending ||
            activeRun !== null ||
            blockedReason !== null ||
            workingDirectory.trim() === '' ||
            !configQuery.data
          }
          title={blockedReason ?? undefined}
        >
          <Play className="h-4 w-4" />
          {launch.isPending
            ? 'Starting…'
            : activeRun
              ? 'Run in progress'
              : 'Run once'}
        </Button>
        {runsQuery.isFetching || detailQuery.isFetching ? (
          <span className="text-xs text-muted-foreground" role="status">
            Refreshing…
          </span>
        ) : null}
      </div>

      {blockedReason ? (
        <p className="mt-2 text-xs text-amber-800">{blockedReason}</p>
      ) : null}
      {configQuery.isError ? (
        <div
          role="alert"
          className="mt-3 flex flex-wrap items-center gap-2 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs text-rose-900"
        >
          <span>Run configuration failed: {configQuery.error.message}</span>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              void configQuery.refetch();
            }}
          >
            Retry
          </Button>
        </div>
      ) : null}
      {error ? (
        <p
          role="alert"
          className="mt-2 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs text-rose-900"
        >
          {error}
        </p>
      ) : null}

      <p className="mt-2 text-[11px] text-muted-foreground">
        Codex uses a workspace-write sandbox. Claude Code uses accept-edits
        permissions and denies other permission prompts. The agent stops with a
        reason instead of waiting for input; no automatic retry is attempted.
        Working-directory and task conflicts are rejected before any process
        starts.
      </p>

      <div className="mt-4 border-t pt-3">
        <div className="flex items-baseline justify-between gap-2">
          <h4 className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
            Run history
          </h4>
          <span className="text-[11px] text-muted-foreground">
            {runs.length} {runs.length === 1 ? 'attempt' : 'attempts'}
          </span>
        </div>
        {runsQuery.isError && !runsQuery.data ? (
          <div
            role="alert"
            className="mt-2 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs text-rose-900"
          >
            Unable to load runs: {runsQuery.error.message}
            <Button
              variant="outline"
              size="sm"
              className="mt-2"
              onClick={() => {
                void runsQuery.refetch();
              }}
            >
              Retry
            </Button>
          </div>
        ) : runs.length === 0 ? (
          <p className="mt-2 text-xs text-muted-foreground">
            No attempts yet. Run history survives dashboard restarts, even when
            an attempt stops without recording a task result.
          </p>
        ) : (
          <div className="mt-2 space-y-1">
            {runs.map((run) => (
              <button
                key={run.id}
                type="button"
                onClick={() => setSelectedRunId(run.id)}
                aria-pressed={run.id === selectedRunId}
                className={cn(
                  'flex w-full flex-wrap items-center gap-2 rounded-md border px-2.5 py-2 text-left text-xs transition-colors',
                  run.id === selectedRunId
                    ? 'border-primary/40 bg-card shadow-sm'
                    : 'border-transparent hover:bg-card',
                )}
              >
                <span className="font-mono text-[11px] text-muted-foreground">
                  #{run.id}
                </span>
                <RunStatusBadge status={run.status} />
                <span className="capitalize">{run.agent}</span>
                <span className="text-muted-foreground">
                  {formatDate(run.started_at)}
                </span>
                <span className="ml-auto text-muted-foreground">
                  {formatDuration(run)}
                </span>
              </button>
            ))}
          </div>
        )}
      </div>

      {selected ? (
        <RunDetailCard
          run={selected}
          detail={
            detailQuery.data?.run.id === selected.id
              ? detailQuery.data
              : null
          }
          loading={detailQuery.isPending && detailQuery.data === undefined}
          error={
            detailQuery.isError && !detailQuery.data
              ? detailQuery.error.message
              : null
          }
          onRetry={() => {
            void detailQuery.refetch();
          }}
          onCancel={() => cancel.mutate(selected.id)}
          cancelling={cancel.isPending}
          projectId={projectId}
          taskId={task.id}
        />
      ) : null}
    </section>
  );
}

function RunDetailCard({
  run,
  detail,
  loading,
  error,
  onRetry,
  onCancel,
  cancelling,
  projectId,
  taskId,
}: {
  run: RunSummary;
  detail: RunDetail | null;
  loading: boolean;
  error: string | null;
  onRetry: () => void;
  onCancel: () => void;
  cancelling: boolean;
  projectId: string;
  taskId: number;
}) {
  return (
    <Card className="mt-3 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm font-medium">Attempt #{run.id}</span>
        <RunStatusBadge status={run.status} />
        {run.status === 'running' ? (
          <Button
            variant="destructive"
            size="sm"
            className="ml-auto"
            onClick={onCancel}
            disabled={cancelling || run.cancel_requested_at !== null}
          >
            <Square className="h-3.5 w-3.5" />
            {run.cancel_requested_at !== null ? 'Cancelling…' : 'Cancel run'}
          </Button>
        ) : null}
      </div>
      <dl className="mt-2 grid gap-2 text-xs sm:grid-cols-2">
        <Detail label="Agent">
          {run.agent === 'codex' ? 'Codex' : 'Claude Code'} ·{' '}
          {run.agent_version}
        </Detail>
        <Detail label="Executable">
          <code className="break-all">{run.agent_executable}</code>
        </Detail>
        <Detail label="Working directory">
          <code className="break-all">{run.working_directory}</code>
        </Detail>
        <Detail label="Input task revision">{run.input_task_revision}</Detail>
        <Detail label="Started">{formatDate(run.started_at)}</Detail>
        <Detail label="Ended">{formatDate(run.ended_at)}</Detail>
        <Detail label="Exit code">{run.exit_code ?? '—'}</Detail>
        <Detail label="Stop reason">{run.stop_reason ?? '—'}</Detail>
      </dl>

      {run.status === 'succeeded' ? (
        <p className="mt-2 rounded-lg border border-emerald-200 bg-emerald-50 px-3 py-2 text-xs text-emerald-900">
          The agent process exited successfully. This is not task completion;
          review the task result and dependency gates before changing status.
        </p>
      ) : null}

      {loading ? (
        <p className="mt-3 text-xs text-muted-foreground" role="status">
          Loading attempt output…
        </p>
      ) : error ? (
        <div
          role="alert"
          className="mt-3 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs text-rose-900"
        >
          Unable to load attempt output: {error}
          <Button
            variant="outline"
            size="sm"
            className="mt-2"
            onClick={onRetry}
          >
            Retry
          </Button>
        </div>
      ) : detail ? (
        <div className="mt-3 space-y-3">
          <div>
            <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
              Progress
            </p>
            {detail.progress ? (
              <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-slate-950/90 p-3 font-mono text-[11px] leading-relaxed text-slate-100">
                {detail.progress}
              </pre>
            ) : (
              <p className="mt-1 text-xs text-muted-foreground">
                {run.status === 'running'
                  ? 'Waiting for agent output…'
                  : 'The runtime did not report recognizable progress output.'}
              </p>
            )}
          </div>

          <div>
            <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
              Task results recorded by this attempt
            </p>
            {detail.results.length === 0 ? (
              <p className="mt-1 text-xs text-muted-foreground">
                None. The attempt remains visible here even without a task
                result.
              </p>
            ) : (
              <div className="mt-1 space-y-2">
                {detail.results.map((result) => (
                  <details
                    key={result.id}
                    className="rounded-lg border bg-card p-2"
                  >
                    <summary className="cursor-pointer text-xs">
                      <span className="font-medium">Result #{result.id}</span>
                      <span className="ml-2 capitalize text-muted-foreground">
                        {result.outcome.replace('_', ' ')} ·{' '}
                        {formatDate(result.recorded_at)}
                      </span>
                    </summary>
                    <div className="mt-2 border-t pt-2 text-sm">
                      <Markdown text={result.evidence} />
                      {result.output_source_ids.length > 0 ? (
                        <p className="mt-2 text-xs text-muted-foreground">
                          Output sources:{' '}
                          {formatIds(result.output_source_ids)}
                        </p>
                      ) : null}
                    </div>
                  </details>
                ))}
              </div>
            )}
          </div>

          <RunRawOutput
            key={run.id}
            projectId={projectId}
            taskId={taskId}
            run={run}
            logTruncated={detail.log_truncated}
          />
        </div>
      ) : null}
    </Card>
  );
}

function RunRawOutput({
  projectId,
  taskId,
  run,
  logTruncated,
}: {
  projectId: string;
  taskId: number;
  run: RunSummary;
  logTruncated: boolean;
}) {
  const [open, setOpen] = useState(false);
  const query = useQuery({
    queryKey: queryKeys.runLog(projectId, taskId, run.id),
    queryFn: () => api.run(projectId, taskId, run.id, true),
    enabled: open,
    // Poll until this query's own snapshot is terminal, then fetch once more
    // only if the user reopens it. The final log is written with the status.
    refetchInterval: (query) =>
      query.state.data?.run.status === 'running' ? 2_000 : false,
  });

  return (
    <details
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
      className="rounded-lg border bg-card p-2"
    >
      <summary className="cursor-pointer text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
        Raw output
        {logTruncated ? ' · earlier output truncated' : ''}
      </summary>
      {!open ? null : query.isPending && !query.data ? (
        <p className="mt-2 text-xs text-muted-foreground" role="status">
          Loading raw output…
        </p>
      ) : query.isError && !query.data ? (
        <div
          role="alert"
          className="mt-2 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs text-rose-900"
        >
          Unable to load raw output: {query.error.message}
          <Button
            variant="outline"
            size="sm"
            className="mt-2"
            onClick={() => {
              void query.refetch();
            }}
          >
            Retry
          </Button>
        </div>
      ) : (
        <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed">
          {query.data?.log || 'No output captured.'}
        </pre>
      )}
    </details>
  );
}

function RunStatusBadge({ status }: { status: RunStatus }) {
  return (
    <Badge variant="outline" className={runStatusClasses[status]}>
      {runStatusLabels[status]}
    </Badge>
  );
}

function Detail({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <Meta label={label} className="break-words text-xs">
      {children}
    </Meta>
  );
}

function runBlockedReason(
  task: Task,
  dependencyState: DependencyState,
): string | null {
  if (dependencyState.unmet_prerequisite_task_ids.length > 0) {
    return `Resolve before-start dependencies first: ${formatIds(
      dependencyState.unmet_prerequisite_task_ids,
    )}.`;
  }
  if (task.kind === 'one_time' && task.status === 'done') {
    return 'Done one-time tasks must be reopened before another attempt.';
  }
  if (task.kind === 'recurring' && task.status !== 'active') {
    return 'Only active recurring tasks can be run.';
  }
  return null;
}

function formatDuration(run: RunSummary): string {
  const start = new Date(run.started_at).getTime();
  const end = run.ended_at
    ? new Date(run.ended_at).getTime()
    : Date.now();
  if (!Number.isFinite(start) || !Number.isFinite(end)) {
    return '—';
  }
  const seconds = Math.max(0, Math.round((end - start) / 1000));
  if (seconds < 60) {
    return `${seconds}s`;
  }
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  if (minutes < 60) {
    return `${minutes}m ${remainder}s`;
  }
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

function formatLimit(seconds: number): string {
  if (seconds % 60 === 0) {
    return `${seconds / 60} min`;
  }
  return `${seconds}s`;
}

function storedValue(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function storedAgent(): RunAgent | null {
  const value = storedValue('dictum.run.agent');
  return value === 'codex' || value === 'claude' ? value : null;
}

function storeValue(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Per-viewer convenience only; the form still works without storage.
  }
}
