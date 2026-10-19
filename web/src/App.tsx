import { useCallback, useEffect, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { RefreshCw, Search } from 'lucide-react';
import { api, ApiError, queryKeys } from './lib/api';
import { formatTime } from './lib/format';
import type { TaskKind } from './lib/types';
import { cn } from './lib/utils';
import { Board } from './components/Board';
import { ProjectChooser } from './components/ProjectChooser';
import { ProjectSections } from './components/ProjectSections';
import { TaskDetail } from './components/TaskDetail';
import { Button } from './components/ui/button';
import { Input } from './components/ui/input';

interface Selection {
  projectId: string | null;
  taskId: number | null;
}

function readSelection(): Selection {
  const params = new URLSearchParams(window.location.search);
  const rawProject = params.get('project');
  const rawTask = params.get('task');
  let taskId: number | null = null;
  if (rawTask !== null) {
    const parsed = Number(rawTask);
    taskId =
      Number.isInteger(parsed) && parsed > 0 ? parsed : Number.NaN;
  }
  return {
    projectId:
      rawProject !== null && rawProject.length > 0 ? rawProject : null,
    taskId,
  };
}

function useSelection() {
  const [selection, setSelection] = useState<Selection>(readSelection);

  useEffect(() => {
    const onPopState = () => setSelection(readSelection());
    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, []);

  const navigate = useCallback(
    (
      projectId: string | null,
      taskId: number | null,
      replace = false,
    ) => {
      const url = new URL(window.location.href);
      if (projectId) {
        url.searchParams.set('project', projectId);
      } else {
        url.searchParams.delete('project');
      }
      if (taskId !== null && Number.isFinite(taskId)) {
        url.searchParams.set('task', String(taskId));
      } else {
        url.searchParams.delete('task');
      }
      if (replace) {
        window.history.replaceState({}, '', url);
      } else {
        window.history.pushState({}, '', url);
      }
      setSelection({ projectId, taskId });
    },
    [],
  );

  return { selection, navigate };
}

export default function App() {
  const { selection, navigate } = useSelection();
  const { projectId, taskId } = selection;
  const queryClient = useQueryClient();
  const [search, setSearch] = useState('');
  const [activeKind, setActiveKind] = useState<TaskKind>('one_time');
  const syncedTaskId = useRef<number | null>(null);

  const projectsQuery = useQuery({
    queryKey: queryKeys.projects,
    queryFn: api.projects,
    refetchInterval: 30_000,
  });

  const tasksQuery = useQuery({
    queryKey: queryKeys.tasks(projectId),
    queryFn: () => {
      if (!projectId) {
        throw new Error('A project must be selected');
      }
      return api.tasks(projectId);
    },
    enabled: projectId !== null,
    refetchInterval: 5_000,
  });

  useEffect(() => {
    setSearch('');
    setActiveKind('one_time');
    syncedTaskId.current = null;
  }, [projectId]);

  const projects = projectsQuery.data?.projects ?? [];
  const project = projects.find((candidate) => candidate.id === projectId) ?? null;
  const tasks = tasksQuery.data?.tasks ?? [];

  useEffect(() => {
    if (taskId === null) {
      syncedTaskId.current = null;
      return;
    }
    if (syncedTaskId.current === taskId) {
      return;
    }
    const selected = tasks.find((task) => task.id === taskId);
    if (selected) {
      setActiveKind(selected.kind);
      syncedTaskId.current = taskId;
    }
  }, [taskId, tasks]);

  const refresh = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.projects });
    if (projectId) {
      void queryClient.invalidateQueries({
        queryKey: ['projects', projectId],
      });
    }
  }, [projectId, queryClient]);

  const openTask = useCallback(
    (id: number) => {
      if (projectId) {
        navigate(projectId, id);
      }
    },
    [navigate, projectId],
  );

  const closeDetail = useCallback(() => {
    if (!projectId) {
      return;
    }
    const previousTask = taskId;
    navigate(projectId, null);
    if (previousTask !== null && Number.isFinite(previousTask)) {
      requestAnimationFrame(() => {
        const card = document.querySelector<HTMLElement>(
          `[data-task-id="${previousTask}"]`,
        );
        (card ?? document.getElementById('project-select'))?.focus();
      });
    }
  }, [navigate, projectId, taskId]);

  const projectNotFound =
    tasksQuery.error instanceof ApiError && tasksQuery.error.status === 404;
  const stale = tasksQuery.isError && tasksQuery.data !== undefined;
  const updatedLabel = tasksQuery.data
    ? tasksQuery.isError
      ? `Stale · last success ${formatTime(tasksQuery.dataUpdatedAt)}`
      : `Updated ${formatTime(tasksQuery.dataUpdatedAt)}`
    : tasksQuery.isError
      ? 'Load failed'
      : 'Loading…';

  return (
    <div className="min-h-screen">
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:absolute focus:left-3 focus:top-3 focus:z-50 focus:rounded-md focus:bg-card focus:px-3 focus:py-2 focus:text-sm focus:shadow"
      >
        Skip to content
      </a>

      <header className="sticky top-0 z-20 border-b bg-background/90 backdrop-blur">
        <div className="mx-auto flex max-w-[1600px] flex-wrap items-center gap-2 px-4 py-3 sm:gap-3 sm:px-6">
          <div className="mr-auto flex items-center gap-2">
            <span
              aria-hidden="true"
              className="grid h-8 w-8 place-items-center rounded-lg bg-primary text-sm font-bold text-primary-foreground"
            >
              D
            </span>
            <span className="text-lg font-semibold tracking-tight">
              Dictum
            </span>
          </div>

          <label htmlFor="project-select" className="sr-only">
            Project
          </label>
          <select
            id="project-select"
            value={projectId ?? ''}
            onChange={(event) =>
              navigate(event.target.value || null, null)
            }
            disabled={projects.length === 0}
            className="h-9 max-w-[20rem] rounded-md border border-input bg-card px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
          >
            <option value="">Choose project…</option>
            {projects.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.name} — {candidate.id}
              </option>
            ))}
          </select>

          {projectId ? (
            <div className="relative">
              <Search
                aria-hidden="true"
                className="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground"
              />
              <label htmlFor="task-search" className="sr-only">
                Search tasks
              </label>
              <Input
                id="task-search"
                type="search"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder="Search title or ID"
                className="w-52 pl-8"
              />
            </div>
          ) : null}

          <span
            className="text-xs tabular-nums text-muted-foreground"
            aria-live="polite"
          >
            {projectId && tasksQuery.data
              ? `${tasks.length} ${tasks.length === 1 ? 'task' : 'tasks'} · `
              : ''}
            {updatedLabel}
          </span>

          <Button
            variant="outline"
            size="sm"
            onClick={refresh}
            disabled={!projectId || tasksQuery.isFetching}
          >
            <RefreshCw
              className={cn(
                'h-4 w-4',
                tasksQuery.isFetching && 'animate-spin',
              )}
            />
            Refresh
          </Button>
        </div>
      </header>

      {stale ? (
        <div
          role="status"
          className="border-b border-amber-200 bg-amber-50 px-4 py-2 text-sm text-amber-900 sm:px-6"
        >
          Refresh failed: {tasksQuery.error?.message}. Showing data from{' '}
          {formatTime(tasksQuery.dataUpdatedAt)}.
        </div>
      ) : null}

      <main
        id="main"
        className="mx-auto w-full max-w-[1600px] px-4 py-6 sm:px-6"
      >
        {projectsQuery.isPending ? (
          <LoadingPanel label="Loading projects…" />
        ) : projectsQuery.isError && !projectsQuery.data ? (
          <ErrorPanel
            title="Unable to load projects"
            message={projectsQuery.error.message}
            onRetry={refresh}
          />
        ) : !projectId ? (
          <ProjectChooser
            projects={projects}
            error={
              projectsQuery.isError
                ? projectsQuery.error.message
                : null
            }
            onSelect={(id) => navigate(id, null)}
            onRetry={refresh}
          />
        ) : tasksQuery.isPending ? (
          <LoadingPanel label="Loading tasks…" />
        ) : tasksQuery.isError && !tasksQuery.data ? (
          projectNotFound ? (
            <ErrorPanel
              title="Project not found"
              message={`No project matches ${projectId}. It may have been removed.`}
              actionLabel="Choose a project"
              onRetry={() => navigate(null, null)}
            />
          ) : (
            <ErrorPanel
              title="Unable to load tasks"
              message={tasksQuery.error.message}
              onRetry={() => {
                void tasksQuery.refetch();
              }}
            />
          )
        ) : (
          <div className="space-y-10">
            <Board
              project={project}
              tasks={tasks}
              search={search}
              activeKind={activeKind}
              onKindChange={setActiveKind}
              selectedTaskId={taskId}
              onSelectTask={openTask}
              onClearSearch={() => setSearch('')}
            />
            <ProjectSections projectId={projectId} />
          </div>
        )}
      </main>

      {projectId && taskId !== null ? (
        <TaskDetail
          projectId={projectId}
          taskId={taskId}
          tasks={tasks}
          onClose={closeDetail}
          onOpenTask={openTask}
        />
      ) : null}
    </div>
  );
}

function LoadingPanel({ label }: { label: string }) {
  return (
    <div
      role="status"
      className="rounded-xl border border-dashed bg-card p-8 text-center text-sm text-muted-foreground"
    >
      {label}
    </div>
  );
}

function ErrorPanel({
  title,
  message,
  onRetry,
  actionLabel = 'Retry',
}: {
  title: string;
  message: string;
  onRetry: () => void;
  actionLabel?: string;
}) {
  return (
    <div
      role="alert"
      className="rounded-xl border border-rose-200 bg-rose-50 p-6 text-rose-900"
    >
      <h2 className="text-base font-semibold">{title}</h2>
      <p className="mt-1 text-sm">{message}</p>
      <Button
        variant="outline"
        size="sm"
        className="mt-3"
        onClick={onRetry}
      >
        {actionLabel}
      </Button>
    </div>
  );
}
