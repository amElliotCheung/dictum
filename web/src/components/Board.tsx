import { useRef, type KeyboardEvent } from 'react';
import { Badge } from './ui/badge';
import { Button } from './ui/button';
import { cn } from '../lib/utils';
import { formatIds, statusLabels } from '../lib/format';
import type {
  Project,
  TaskKind,
  TaskStatus,
  TaskSummary,
} from '../lib/types';

interface ColumnDefinition {
  status: TaskStatus;
  label: string;
  border: string;
  dot: string;
}

const oneTimeColumns: ColumnDefinition[] = [
  { status: 'todo', label: 'Todo', border: 'border-t-sky-400', dot: 'bg-sky-400' },
  {
    status: 'in_progress',
    label: 'In progress',
    border: 'border-t-amber-400',
    dot: 'bg-amber-400',
  },
  {
    status: 'blocked',
    label: 'Blocked',
    border: 'border-t-rose-400',
    dot: 'bg-rose-400',
  },
  {
    status: 'done',
    label: 'Done',
    border: 'border-t-emerald-400',
    dot: 'bg-emerald-400',
  },
];

const recurringColumns: ColumnDefinition[] = [
  {
    status: 'active',
    label: 'Active',
    border: 'border-t-violet-400',
    dot: 'bg-violet-400',
  },
  {
    status: 'paused',
    label: 'Paused',
    border: 'border-t-slate-400',
    dot: 'bg-slate-400',
  },
];

const tabs: { kind: TaskKind; label: string }[] = [
  { kind: 'one_time', label: 'One-time' },
  { kind: 'recurring', label: 'Recurring' },
];

export function Board({
  project,
  tasks,
  search,
  activeKind,
  onKindChange,
  selectedTaskId,
  onSelectTask,
  onClearSearch,
}: {
  project: Project | null;
  tasks: TaskSummary[];
  search: string;
  activeKind: TaskKind;
  onKindChange: (kind: TaskKind) => void;
  selectedTaskId: number | null;
  onSelectTask: (taskId: number) => void;
  onClearSearch: () => void;
}) {
  const tabRefs = useRef<Record<TaskKind, HTMLButtonElement | null>>({
    one_time: null,
    recurring: null,
  });
  const oneTime = tasks.filter((task) => task.kind === 'one_time');
  const recurring = tasks.filter((task) => task.kind === 'recurring');
  const counts: Record<TaskKind, number> = {
    one_time: oneTime.length,
    recurring: recurring.length,
  };
  const kindTasks = activeKind === 'one_time' ? oneTime : recurring;
  const columns = activeKind === 'one_time' ? oneTimeColumns : recurringColumns;
  const kindLabel = activeKind === 'one_time' ? 'one-time' : 'recurring';
  const filtering = search.trim().length > 0;
  const query = search.trim().toLowerCase();
  const visible = query
    ? kindTasks.filter(
        (task) =>
          String(task.id).includes(query) ||
          task.title.toLowerCase().includes(query),
      )
    : kindTasks;

  const onTabKeyDown = (
    event: KeyboardEvent<HTMLButtonElement>,
    kind: TaskKind,
  ) => {
    let next: TaskKind | null = null;
    switch (event.key) {
      case 'ArrowRight':
      case 'ArrowLeft':
        next = kind === 'one_time' ? 'recurring' : 'one_time';
        break;
      case 'Home':
        next = 'one_time';
        break;
      case 'End':
        next = 'recurring';
        break;
    }
    if (next !== null) {
      event.preventDefault();
      onKindChange(next);
      requestAnimationFrame(() => tabRefs.current[next]?.focus());
    }
  };

  return (
    <div className="space-y-6">
      {project ? (
        <div>
          <h1 className="text-xl font-semibold tracking-tight">{project.name}</h1>
          <p className="mt-1 max-w-3xl text-sm text-muted-foreground">
            {project.description}
          </p>
          <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
            {project.id}
          </p>
        </div>
      ) : null}

      <div>
        <div
          role="tablist"
          aria-label="Task kind"
          className="inline-flex items-center gap-1 rounded-lg border bg-muted/50 p-1"
        >
          {tabs.map(({ kind, label }) => {
            const selected = activeKind === kind;
            return (
              <button
                key={kind}
                ref={(element) => {
                  tabRefs.current[kind] = element;
                }}
                type="button"
                role="tab"
                id={`${kind}-tab`}
                aria-selected={selected}
                aria-controls="task-panel"
                tabIndex={selected ? 0 : -1}
                onClick={() => onKindChange(kind)}
                onKeyDown={(event) => onTabKeyDown(event, kind)}
                className={cn(
                  'inline-flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                  selected
                    ? 'bg-card text-foreground shadow-sm'
                    : 'text-muted-foreground hover:text-foreground',
                )}
              >
                {label}
                <span
                  className={cn(
                    'rounded-full px-1.5 py-0.5 text-[11px] tabular-nums',
                    selected
                      ? 'bg-muted text-foreground'
                      : 'bg-card/70 text-muted-foreground',
                  )}
                >
                  {counts[kind]}
                </span>
              </button>
            );
          })}
        </div>

        <div
          className="space-y-3"
          role="tabpanel"
          id="task-panel"
          aria-labelledby={`${activeKind}-tab`}
        >
          <div className="mt-3 flex flex-wrap items-center justify-between gap-2">
            <p className="text-sm text-muted-foreground" aria-live="polite">
              Showing{' '}
              <strong className="text-foreground">{visible.length}</strong> of{' '}
              <strong className="text-foreground">{kindTasks.length}</strong>{' '}
              {kindLabel} {kindTasks.length === 1 ? 'task' : 'tasks'}
              {filtering ? ` matching “${search.trim()}”` : ''}.
            </p>
            {filtering ? (
              <Button variant="outline" size="sm" onClick={onClearSearch}>
                Clear search
              </Button>
            ) : null}
          </div>

          {filtering && visible.length === 0 && kindTasks.length > 0 ? (
            <p className="rounded-lg border border-dashed bg-card p-6 text-center text-sm text-muted-foreground">
              No {kindLabel} tasks match “{search.trim()}”. Clear the search to
              see all {kindTasks.length} {kindLabel} tasks.
            </p>
          ) : null}

          <Lifecycle
            id={activeKind}
            title={activeKind === 'one_time' ? 'One-time tasks' : 'Recurring tasks'}
            hint={
              activeKind === 'one_time'
                ? 'Todo, in progress, blocked and done are lifecycle states. Dependency readiness is shown separately on each card.'
                : 'Active means eligible for externally authorized work. It is not a running agent or an installed schedule; recurring tasks never reach done.'
            }
            columns={columns}
            tasks={visible}
            selectedTaskId={selectedTaskId}
            onSelectTask={onSelectTask}
          />
        </div>
      </div>
    </div>
  );
}

function Lifecycle({
  id,
  title,
  hint,
  columns,
  tasks,
  selectedTaskId,
  onSelectTask,
}: {
  id: string;
  title: string;
  hint: string;
  columns: ColumnDefinition[];
  tasks: TaskSummary[];
  selectedTaskId: number | null;
  onSelectTask: (taskId: number) => void;
}) {
  return (
    <section aria-labelledby={`${id}-heading`}>
      <div className="mb-1 flex flex-wrap items-baseline justify-between gap-2">
        <h2 id={`${id}-heading`} className="text-base font-semibold">
          {title}
        </h2>
        <span className="text-xs text-muted-foreground">
          {tasks.length} shown
        </span>
      </div>
      <p className="mb-3 max-w-4xl text-xs text-muted-foreground">{hint}</p>
      <div
        className={cn(
          'grid gap-3',
          columns.length === 4
            ? 'sm:grid-cols-2 xl:grid-cols-4'
            : 'sm:grid-cols-2',
        )}
      >
        {columns.map((column) => {
          const columnTasks = tasks.filter(
            (task) => task.status === column.status,
          );
          return (
            <div
              key={column.status}
              className={cn(
                'flex min-h-32 flex-col rounded-xl border border-t-4 bg-muted/30',
                column.border,
              )}
            >
              <div className="flex items-center justify-between gap-2 px-3 py-2">
                <h3 className="flex items-center gap-2 text-sm font-semibold">
                  <span
                    aria-hidden="true"
                    className={cn('h-2 w-2 rounded-full', column.dot)}
                  />
                  {column.label}
                </h3>
                <span
                  className="rounded-full bg-card px-2 py-0.5 text-xs text-muted-foreground"
                  aria-label={`${columnTasks.length} tasks`}
                >
                  {columnTasks.length}
                </span>
              </div>
              <div className="flex flex-1 flex-col gap-2 px-2 pb-2">
                {columnTasks.length === 0 ? (
                  <p className="rounded-lg border border-dashed px-3 py-4 text-center text-xs text-muted-foreground">
                    No tasks
                  </p>
                ) : (
                  columnTasks.map((task) => (
                    <TaskCard
                      key={task.id}
                      task={task}
                      selected={task.id === selectedTaskId}
                      onSelect={onSelectTask}
                    />
                  ))
                )}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}

function TaskCard({
  task,
  selected,
  onSelect,
}: {
  task: TaskSummary;
  selected: boolean;
  onSelect: (taskId: number) => void;
}) {
  const dependencies = task.dependency_state;
  return (
    <button
      type="button"
      data-task-id={task.id}
      onClick={() => onSelect(task.id)}
      aria-label={`Task ${task.id}: ${task.title}. Status ${statusLabels[task.status]}.`}
      className={cn(
        'group w-full rounded-lg border bg-card p-3 text-left shadow-sm transition hover:border-primary/40 hover:shadow-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        selected && 'border-primary ring-2 ring-primary',
      )}
    >
      <span className="flex items-center justify-between gap-2">
        <span className="text-xs font-semibold tabular-nums text-muted-foreground">
          #{task.id}
        </span>
        <Badge variant={task.kind === 'recurring' ? 'secondary' : 'outline'}>
          {task.kind === 'recurring' ? 'Recurring' : 'One-time'}
        </Badge>
      </span>
      <span className="mt-2 block text-sm font-medium leading-5">
        {task.title}
      </span>
      {task.progress ? (
        <span className="mt-1 line-clamp-2 block text-xs text-muted-foreground">
          {task.progress}
        </span>
      ) : null}
      {dependencies.unmet_prerequisite_task_ids.length > 0 ||
      dependencies.unmet_completion_task_ids.length > 0 ||
      (task.status === 'todo' && dependencies.ready_to_start) ||
      task.trigger_event ? (
        <span className="mt-2 flex flex-wrap gap-1">
          {dependencies.unmet_prerequisite_task_ids.length > 0 ? (
            <Badge
              variant="outline"
              className="border-amber-300 bg-amber-50 text-amber-800"
            >
              Start blocked: {formatIds(dependencies.unmet_prerequisite_task_ids)}
            </Badge>
          ) : null}
          {dependencies.unmet_completion_task_ids.length > 0 ? (
            <Badge
              variant="outline"
              className="border-rose-300 bg-rose-50 text-rose-800"
            >
              {task.kind === 'recurring' ? 'Success blocked' : 'Done blocked'}: {formatIds(dependencies.unmet_completion_task_ids)}
            </Badge>
          ) : null}
          {task.status === 'todo' && dependencies.ready_to_start ? (
            <Badge
              variant="outline"
              className="border-emerald-300 bg-emerald-50 text-emerald-800"
            >
              Ready to start
            </Badge>
          ) : null}
          {task.trigger_event ? (
            <Badge variant="outline">Trigger: {task.trigger_event}</Badge>
          ) : null}
        </span>
      ) : null}
      <span className="sr-only">
        Lifecycle status: {statusLabels[task.status]}. Dependency readiness is
        separate from lifecycle status.
      </span>
    </button>
  );
}
