import { useEffect, useRef, useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { X } from 'lucide-react';
import { api, ApiError, queryKeys } from '../lib/api';
import { formatDate, formatKind, statusLabels } from '../lib/format';
import type { Task, TaskContext, TaskStatus, TaskSummary } from '../lib/types';
import { Markdown } from '../lib/markdown';
import { Badge } from './ui/badge';
import { Button } from './ui/button';
import { Meta } from './ui/meta';
import { DecisionCard } from './DecisionCard';
import { SourceBody } from './SourceCard';
import { TaskComments } from './TaskComments';

export function TaskDetail({ projectId, taskId, tasks, onClose, onOpenTask }: {
  projectId: string;
  taskId: number;
  tasks: TaskSummary[];
  onClose: () => void;
  onOpenTask: (taskId: number) => void;
}) {
  const closeRef = useRef<HTMLButtonElement>(null);
  const validTask = Number.isInteger(taskId) && taskId > 0;
  useEffect(() => { closeRef.current?.focus(); }, [projectId, taskId]);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.preventDefault(); onClose(); } };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [onClose]);
  const contextQuery = useQuery({
    queryKey: queryKeys.context(projectId, taskId),
    queryFn: () => api.context(projectId, taskId),
    enabled: validTask,
    refetchInterval: 5_000,
  });
  const context = contextQuery.data;
  const notFound = contextQuery.error instanceof ApiError && contextQuery.error.status === 404;
  return (
    <>
      <div aria-hidden="true" onClick={onClose} className="fixed inset-0 z-20 bg-slate-950/20 backdrop-blur-[1px]" />
      <aside role="dialog" aria-modal="false" aria-labelledby="task-detail-title" tabIndex={-1} className="fixed inset-y-0 right-0 z-30 flex w-full max-w-2xl flex-col border-l bg-card shadow-2xl">
        <div className="flex items-start justify-between gap-3 border-b px-4 py-3">
          <div className="min-w-0">
            <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Task {validTask ? `#${taskId}` : ''}</p>
            <h2 id="task-detail-title" className="truncate text-lg font-semibold">{context?.task.title ?? (validTask ? 'Loading task…' : 'Invalid task selection')}</h2>
          </div>
          <Button ref={closeRef} variant="ghost" size="icon" onClick={onClose} aria-label="Close task details"><X className="h-4 w-4" /></Button>
        </div>
        <div className="flex-1 overflow-y-auto px-4 py-4">
          {!validTask ? (
            <PanelState title="Invalid task selection" message="The task ID in this URL is not a positive integer. Close this panel and choose a task from the board." />
          ) : contextQuery.isPending ? <LoadingDetail /> : !context ? (
            <PanelState title={notFound ? 'Task not found' : 'Unable to load task'} message={notFound ? `No task #${taskId} exists in this project.` : contextQuery.error?.message ?? 'Unknown error'} onRetry={() => { void contextQuery.refetch(); }} />
          ) : (
            <>
              {contextQuery.isError ? <RefreshNotice message={contextQuery.error.message} onRetry={() => { void contextQuery.refetch(); }} /> : null}
              <TaskContent key={`${projectId}:${taskId}`} context={context} tasks={tasks} onOpenTask={onOpenTask} />
            </>
          )}
        </div>
      </aside>
    </>
  );
}

function TaskContent({ context, tasks, onOpenTask }: { context: TaskContext; tasks: TaskSummary[]; onOpenTask: (taskId: number) => void }) {
  const { task, dependency_state, parent_task } = context;
  return (
    <div className="space-y-6">
      <dl className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Meta label="Kind">{formatKind(task.kind)}</Meta>
        <Meta label="Status"><StatusBadge status={task.status} /></Meta>
        <Meta label="Priority"><span className="capitalize">{task.priority}</span></Meta>
        <Meta label="Version">{task.revision}</Meta>
        <Meta label="Trigger">{task.trigger_event ?? '—'}</Meta>
        <Meta label="Created">{formatDate(task.created_at)}</Meta>
        <Meta label="Updated">{formatDate(task.updated_at)}</Meta>
      </dl>
      {parent_task ? (
        <section className="rounded-lg border bg-muted/20 p-3" aria-label="Parent task">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-xs font-medium text-muted-foreground">Parent task · version {parent_task.revision}</p>
            <Button variant="outline" size="sm" onClick={() => onOpenTask(parent_task.id)}>Open parent #{parent_task.id}</Button>
          </div>
          <p className="mt-2 text-sm font-semibold">{parent_task.title}</p>
          <details className="mt-3"><summary className="cursor-pointer text-xs font-medium">Read parent brief</summary><div className="mt-3"><TaskBrief task={parent_task} /></div></details>
        </section>
      ) : null}
      {task.kind === 'recurring' ? <p className="rounded-lg border border-violet-200 bg-violet-50 px-3 py-2 text-xs text-violet-900">Active is eligibility for externally authorized work. Recurring tasks never reach done.</p> : null}
      {task.progress ? <Section title="Progress"><Markdown text={task.progress} /></Section> : null}
      <Section title="Brief"><TaskBrief task={task} /></Section>
      {task.parent_task_id === null ? <Subtasks projectId={task.project_id} taskId={task.id} tasks={tasks} onOpenTask={onOpenTask} /> : null}
      <Section title="Dependencies">
        <div className="grid gap-4 sm:grid-cols-2">
          <DependencyGroup title="Before start" ids={task.prerequisite_task_ids} unmet={dependency_state.unmet_prerequisite_task_ids} tasks={tasks} onOpenTask={onOpenTask} />
          <DependencyGroup title={task.kind === 'recurring' ? 'Before success' : 'Before done'} ids={task.completion_task_ids} unmet={dependency_state.unmet_completion_task_ids} tasks={tasks} onOpenTask={onOpenTask} />
        </div>
        <p className="mt-3 text-xs text-muted-foreground">{task.kind === 'recurring' ? 'All dependencies must be done before recording success or no change; failure reports remain allowed.' : dependency_state.ready_to_start ? 'Ready to start: all before-start gates are done. Completion gates still apply before done.' : 'Lifecycle status and dependency readiness are separate. A todo task can still be blocked by its before-start gates.'}</p>
      </Section>
      <TaskComments context={context} />
      <Section title="Current decisions">
        {context.current_decisions.length ? <div className="space-y-2">{context.current_decisions.map((decision) => <DecisionCard key={decision.id} decision={decision} />)}</div> : <p className="text-sm text-muted-foreground">No current decisions for this project.</p>}
      </Section>
      <Section title="Context sources">
        <p className="mb-3 text-xs text-muted-foreground">Inline documents are returned in full. External locators are never fetched automatically.</p>
        {context.sources.length ? (
          <div className="space-y-2">{context.sources.map((source) => (
            <details key={source.id} className="rounded-lg border bg-card">
              <summary className="cursor-pointer px-3 py-2 text-sm font-medium">{source.name}<span className="ml-2 text-xs font-normal text-muted-foreground">{source.context_policy === 'always' ? 'always included' : 'when relevant'} · #{source.id}</span></summary>
              <div className="border-t p-3"><SourceBody source={source} /></div>
            </details>
          ))}</div>
        ) : <p className="text-sm text-muted-foreground">No context sources for this task.</p>}
      </Section>
    </div>
  );
}

function TaskBrief({ task }: { task: Task }) {
  return (
    <div className="space-y-4">
      <div><Subheading>Problem or goal</Subheading><Markdown text={task.problem_or_goal} /></div>
      <div><Subheading>Design</Subheading><Markdown text={task.design} /></div>
      <div><Subheading>Acceptance criteria</Subheading><ol className="list-decimal space-y-2 pl-5">{task.acceptance_criteria.map((criterion, index) => <li key={index}><Markdown text={criterion} /></li>)}</ol></div>
      {task.alternatives_rejected ? <div><Subheading>Alternatives rejected</Subheading><Markdown text={task.alternatives_rejected} /></div> : null}
    </div>
  );
}

function Subtasks({ projectId, taskId, tasks, onOpenTask }: { projectId: string; taskId: number; tasks: TaskSummary[]; onOpenTask: (taskId: number) => void }) {
  const childrenQuery = useQuery({ queryKey: queryKeys.children(projectId, taskId), queryFn: () => api.tasks(projectId, taskId), refetchInterval: 5_000 });
  return (
    <Section title="Subtasks">
      {childrenQuery.isError ? <RefreshNotice message={childrenQuery.error.message} onRetry={() => { void childrenQuery.refetch(); }} label="Subtasks" /> : null}
      {childrenQuery.isPending ? <p role="status" className="text-sm text-muted-foreground">Loading subtasks…</p> : null}
      {childrenQuery.data?.tasks.length === 0 ? <p className="text-sm text-muted-foreground">No subtasks.</p> : null}
      <div className="space-y-3">{(childrenQuery.data?.tasks ?? []).map((child) => <Subtask key={child.id} projectId={projectId} task={child} tasks={tasks} onOpenTask={onOpenTask} />)}</div>
    </Section>
  );
}

function Subtask({ projectId, task, tasks, onOpenTask }: { projectId: string; task: TaskSummary; tasks: TaskSummary[]; onOpenTask: (taskId: number) => void }) {
  const [open, setOpen] = useState(false);
  const contextQuery = useQuery({ queryKey: queryKeys.context(projectId, task.id), queryFn: () => api.context(projectId, task.id), enabled: open, refetchInterval: open ? 5_000 : false, retry: false });
  const contentId = `subtask-content-${task.id}`;
  return (
    <div className="overflow-hidden rounded-lg border bg-card">
      <button type="button" aria-expanded={open} aria-controls={contentId} onClick={() => setOpen((value) => !value)} className="flex w-full flex-wrap items-center justify-between gap-2 p-3 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring">
        <span className="min-w-0 flex-1"><span className="text-xs text-muted-foreground">#{task.id} · {formatKind(task.kind)} · {task.priority} priority</span><span className="mt-1 block text-sm font-medium">{task.title}</span></span>
        <StatusBadge status={task.status} /><span className="text-xs text-muted-foreground">{open ? 'Collapse' : 'Expand'}</span>
      </button>
      {open ? (
        <div id={contentId} className="border-t p-3">
          {contextQuery.isError ? <RefreshNotice message={contextQuery.error.message} onRetry={() => { void contextQuery.refetch(); }} label={`Subtask #${task.id}`} /> : null}
          {contextQuery.isPending ? <LoadingDetail /> : contextQuery.data ? <TaskContent context={contextQuery.data} tasks={tasks} onOpenTask={onOpenTask} /> : null}
        </div>
      ) : null}
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return <section className="border-t pt-5"><h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">{title}</h3>{children}</section>;
}
function Subheading({ children }: { children: ReactNode }) {
  return <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{children}</h4>;
}
const statusClasses: Record<TaskStatus, string> = {
  todo: 'border-sky-300 bg-sky-50 text-sky-800', in_progress: 'border-amber-300 bg-amber-50 text-amber-800',
  blocked: 'border-rose-300 bg-rose-50 text-rose-800', done: 'border-emerald-300 bg-emerald-50 text-emerald-800',
  active: 'border-violet-300 bg-violet-50 text-violet-800', paused: 'border-slate-300 bg-slate-50 text-slate-700',
};
function StatusBadge({ status }: { status: TaskStatus }) {
  return <Badge variant="outline" className={statusClasses[status]}>{statusLabels[status]}</Badge>;
}
function DependencyGroup({ title, ids, unmet, tasks, onOpenTask }: { title: string; ids: number[]; unmet: number[]; tasks: TaskSummary[]; onOpenTask: (taskId: number) => void }) {
  return (
    <div><h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">{title}</h4>
      {ids.length ? <div className="mt-1 flex flex-wrap gap-2">{ids.map((id) => <Button key={id} variant="outline" size="sm" className={unmet.includes(id) ? 'border-rose-300 bg-rose-50 text-rose-800' : undefined} onClick={() => onOpenTask(id)} title={tasks.find((candidate) => candidate.id === id)?.title}>#{id}{unmet.includes(id) ? ' · unmet' : ''}</Button>)}</div> : <p className="mt-1 text-sm text-muted-foreground">None</p>}
    </div>
  );
}
function LoadingDetail() {
  return <div className="space-y-3" role="status" aria-label="Loading task details">{Array.from({ length: 4 }).map((_, index) => <div key={index} className="h-16 animate-pulse rounded-lg bg-muted" />)}</div>;
}
function RefreshNotice({ message, onRetry, label = 'Task' }: { message: string; onRetry: () => void; label?: string }) {
  return <div role="status" className="mb-3 rounded-lg border border-amber-200 bg-amber-50 p-3 text-xs text-amber-900"><p>{label} refresh failed: {message}. Any previously loaded content is retained.</p><Button variant="outline" size="sm" className="mt-2" onClick={onRetry}>Retry {label.toLowerCase()}</Button></div>;
}
function PanelState({ title, message, onRetry }: { title: string; message: string; onRetry?: () => void }) {
  return <div className="rounded-xl border border-dashed bg-muted/40 p-6"><h3 className="text-sm font-semibold">{title}</h3><p className="mt-1 text-sm text-muted-foreground">{message}</p>{onRetry ? <Button variant="outline" size="sm" className="mt-3" onClick={onRetry}>Retry</Button> : null}</div>;
}
