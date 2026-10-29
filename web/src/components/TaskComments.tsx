import { useRef, useState, type FormEvent, type RefObject } from 'react';
import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { api, queryKeys } from '../lib/api';
import {
  clearDraft, draftKey, readDraft, updateDraft, useCommentDraft,
  type CommentDraft,
} from '../lib/comment-drafts';
import { formatDate } from '../lib/format';
import { Markdown } from '../lib/markdown';
import type { Outcome, Task, TaskComment, TaskContext } from '../lib/types';
import { Badge } from './ui/badge';
import { Button } from './ui/button';
import { Card } from './ui/card';
import { SourceCard } from './SourceCard';

export function TaskComments({ context }: { context: TaskContext }) {
  const { task } = context;
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const key = draftKey(task.project_id, task.id);
  const draft = useCommentDraft(key);
  const commentsQuery = useInfiniteQuery({
    queryKey: queryKeys.comments(task.project_id, task.id),
    queryFn: ({ pageParam }) => api.comments(task.project_id, task.id, pageParam),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (page) => page.next_before_id ?? undefined,
    refetchInterval: 5_000,
  });
  const comments = new Map<number, TaskComment>();
  for (const comment of [
    ...context.recent_comments,
    context.latest_execution,
    context.latest_successful_execution,
  ]) {
    if (comment) comments.set(comment.id, comment);
  }
  for (const page of commentsQuery.data?.pages ?? []) {
    for (const comment of page.comments) comments.set(comment.id, comment);
  }
  const timeline = [...comments.values()].sort((left, right) => right.id - left.id);
  const reply = (comment: TaskComment) => {
    const draft = readDraft(key);
    if (draft.sending) return;
    updateDraft(key, {
      replyToCommentId: comment.id,
      taskRevision: draft.taskRevision ?? task.revision,
      error: null,
      notice: null,
    });
    textareaRef.current?.focus();
  };

  return (
    <section className="border-t pt-5" aria-label={`Comments for task ${task.id}`}>
      <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
        Comments
      </h3>
      <CommentComposer task={task} textareaRef={textareaRef} />
      <p className="mb-3 mt-4 text-xs text-muted-foreground">
        Newest first. Execution results are comments with an outcome and optional output sources.
      </p>
      {commentsQuery.isError ? (
        <div role={timeline.length ? 'status' : 'alert'} className="mb-3 rounded-lg border border-amber-200 bg-amber-50 p-3 text-xs text-amber-900">
          <p>Comments could not refresh: {commentsQuery.error.message}. {timeline.length ? 'Showing the comments already loaded.' : ''}</p>
          <Button variant="outline" size="sm" className="mt-2" onClick={() => { void commentsQuery.refetch(); }} disabled={commentsQuery.isFetching}>
            Retry comments
          </Button>
        </div>
      ) : null}
      {timeline.length === 0 ? (
        <p className="text-sm text-muted-foreground" role={commentsQuery.isPending ? 'status' : undefined}>
          {commentsQuery.isPending ? 'Loading comments…' : 'No comments recorded.'}
        </p>
      ) : (
        <div className="space-y-3">
          {timeline.map((comment) => (
            <CommentCard key={comment.id} comment={comment} task={task} comments={comments} latestExecution={comment.id === context.latest_execution?.id} latestSuccessfulExecution={comment.id === context.latest_successful_execution?.id} replyDisabled={draft.sending} onReply={() => reply(comment)} />
          ))}
        </div>
      )}
      {commentsQuery.hasNextPage ? (
        <Button variant="outline" size="sm" className="mt-3" onClick={() => { void commentsQuery.fetchNextPage(); }} disabled={commentsQuery.isFetchingNextPage}>
          {commentsQuery.isFetchingNextPage ? 'Loading…' : 'Load older comments'}
        </Button>
      ) : null}
    </section>
  );
}

function CommentComposer({ task, textareaRef }: { task: Task; textareaRef: RefObject<HTMLTextAreaElement | null> }) {
  const key = draftKey(task.project_id, task.id);
  const draft = useCommentDraft(key);
  const queryClient = useQueryClient();
  const sourcesQuery = useQuery({
    queryKey: queryKeys.projectSources(task.project_id),
    queryFn: () => api.sources(task.project_id),
    enabled: draft.outcome !== '',
    refetchInterval: draft.outcome !== '' ? 5_000 : false,
  });
  const edit = (change: Partial<CommentDraft>) => {
    updateDraft(key, { ...change, taskRevision: draft.taskRevision ?? task.revision, error: null, notice: null });
  };
  const bodyLength = Array.from(draft.body).length;
  const revision = draft.taskRevision ?? task.revision;
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const submitted = readDraft(key);
    if (submitted.sending || !submitted.body.trim() || submitted.taskRevision === null || Array.from(submitted.body).length > 4_000) return;
    const projectId = task.project_id;
    const taskId = task.id;
    const payload = {
      task_revision: submitted.taskRevision,
      body: submitted.body,
      reply_to_comment_id: submitted.replyToCommentId,
      execution_result: submitted.outcome ? {
        outcome: submitted.outcome,
        output_source_ids: submitted.outputSourceIds,
      } : null,
    };
    updateDraft(key, { sending: true, error: null, notice: null });
    try {
      const comment = await api.recordComment(projectId, taskId, payload);
      clearDraft(key, `Comment #${comment.id} posted to task #${taskId}.`);
      void queryClient.invalidateQueries({ queryKey: queryKeys.comments(projectId, taskId) });
      void queryClient.invalidateQueries({ queryKey: queryKeys.context(projectId, taskId) });
    } catch (error) {
      updateDraft(key, {
        sending: false,
        error: `${error instanceof Error ? error.message : 'Unable to post comment'}. Your draft is saved. Check the timeline before posting again if the request may have completed.`,
      });
    }
  };
  const textareaId = `comment-body-${task.id}`;
  return (
    <form onSubmit={(event) => { void submit(event); }} className="rounded-lg border bg-muted/20 p-3">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <label htmlFor={textareaId} className="text-sm font-medium">Add a comment</label>
        <span className="text-xs text-muted-foreground">Task version {revision}{revision === task.revision ? ' · current' : ' · older'}</span>
      </div>
      {draft.replyToCommentId !== null ? (
        <div className="mb-2 flex items-center justify-between gap-2 text-xs">
          <span>Replying to comment #{draft.replyToCommentId}</span>
          <Button variant="ghost" size="sm" disabled={draft.sending} onClick={() => edit({ replyToCommentId: null })}>Remove reply</Button>
        </div>
      ) : null}
      <textarea
        id={textareaId}
        ref={textareaRef}
        value={draft.body}
        onChange={(event) => edit({ body: event.target.value })}
        disabled={draft.sending}
        rows={4}
        placeholder="Share context, ask a question, or report an execution result…"
        aria-describedby={`${textareaId}-version ${textareaId}-count`}
        className="w-full resize-y rounded-md border border-input bg-card px-3 py-2 text-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
      />
      <div id={`${textareaId}-version`} className="mt-2 text-xs text-muted-foreground">
        {draft.taskRevision !== null && draft.taskRevision !== task.revision ? (
          <>
            <p>This draft still refers to task version {draft.taskRevision}. The task is now version {task.revision}.</p>
            <Button variant="outline" size="sm" className="mt-2" disabled={draft.sending} onClick={() => updateDraft(key, { taskRevision: task.revision, error: null, notice: null })}>
              Use current version {task.revision}
            </Button>
          </>
        ) : 'The task version is captured when you start this draft.'}
      </div>
      <div className="mt-3">
        <label htmlFor={`comment-outcome-${task.id}`} className="mb-1 block text-xs font-medium">Execution result (optional)</label>
        <select id={`comment-outcome-${task.id}`} value={draft.outcome} disabled={draft.sending} onChange={(event) => edit({ outcome: event.target.value as Outcome | '' })} className="h-9 w-full rounded-md border border-input bg-card px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
          <option value="">Comment only</option>
          <option value="success">Success</option>
          <option value="no_change">No change</option>
          <option value="failure">Failure</option>
        </select>
      </div>
      {draft.outcome ? (
        <fieldset className="mt-3" disabled={draft.sending}>
          <legend className="text-xs font-medium">Output sources (optional)</legend>
          {sourcesQuery.isError ? (
            <div className="mt-2 text-xs text-destructive" role="alert">
              <p>Could not refresh project sources. Saved selections are retained.</p>
              <Button variant="outline" size="sm" className="mt-1" onClick={() => { void sourcesQuery.refetch(); }}>Retry sources</Button>
            </div>
          ) : null}
          {sourcesQuery.isPending ? <p className="mt-1 text-xs text-muted-foreground">Loading sources…</p> : null}
          <div className="mt-1 max-h-40 space-y-2 overflow-y-auto">
            {(sourcesQuery.data?.sources ?? []).map((source) => (
              <label key={source.id} className="flex items-start gap-2 text-xs">
                <input type="checkbox" checked={draft.outputSourceIds.includes(source.id)} onChange={(event) => edit({ outputSourceIds: event.target.checked ? [...draft.outputSourceIds, source.id] : draft.outputSourceIds.filter((id) => id !== source.id) })} className="mt-0.5" />
                <span>#{source.id} · {source.name}</span>
              </label>
            ))}
          </div>
          {!sourcesQuery.isPending && sourcesQuery.data?.sources.length === 0 ? <p className="mt-1 text-xs text-muted-foreground">No project sources available.</p> : null}
          {draft.outputSourceIds.length ? <p className="mt-2 text-xs text-muted-foreground">Selected: {draft.outputSourceIds.map((id) => `#${id}`).join(', ')}</p> : null}
        </fieldset>
      ) : null}
      {draft.error ? <p role="alert" className="mt-3 text-xs text-destructive">{draft.error}</p> : null}
      {draft.notice ? <p role="status" className="mt-3 text-xs text-emerald-800">{draft.notice}</p> : null}
      <div className="mt-3 flex flex-wrap items-center justify-between gap-2">
        <span id={`${textareaId}-count`} className={`text-xs ${bodyLength > 4_000 ? 'text-destructive' : 'text-muted-foreground'}`}>{bodyLength.toLocaleString()} / 4,000 characters</span>
        <div className="flex gap-2">
          <Button variant="ghost" size="sm" disabled={draft.sending || draft.taskRevision === null} onClick={() => clearDraft(key)}>Discard draft</Button>
          <Button type="submit" size="sm" disabled={draft.sending || !draft.body.trim() || bodyLength > 4_000}>{draft.sending ? 'Posting…' : 'Post comment'}</Button>
        </div>
      </div>
    </form>
  );
}

function CommentCard({ comment, task, comments, latestExecution, latestSuccessfulExecution, replyDisabled, onReply }: { comment: TaskComment; task: Task; comments: ReadonlyMap<number, TaskComment>; latestExecution: boolean; latestSuccessfulExecution: boolean; replyDisabled: boolean; onReply: () => void }) {
  const current = comment.task_revision === task.revision;
  return (
    <Card className="p-3" id={`task-${task.id}-comment-${comment.id}`}>
      <div className="flex flex-wrap items-center gap-2">
        {comment.execution_result ? <OutcomeBadge outcome={comment.execution_result.outcome} /> : <Badge variant="outline">Comment</Badge>}
        <span className="text-xs text-muted-foreground">#{comment.id} · {formatDate(comment.recorded_at)}</span>
        <Badge variant="outline">Version {comment.task_revision} · {current ? 'current' : comment.task_revision < task.revision ? 'older' : 'refresh pending'}</Badge>
        {latestExecution ? <Badge variant="secondary">Latest execution</Badge> : null}
        {latestSuccessfulExecution ? <Badge variant="secondary">Latest successful execution</Badge> : null}
      </div>
      {comment.reply_to_comment_id !== null ? <ReplyReference task={task} commentId={comment.reply_to_comment_id} known={comments.get(comment.reply_to_comment_id)} /> : null}
      <div className="mt-2"><Markdown text={comment.body} /></div>
      <CommentOutputs comment={comment} />
      {comment.execution_result ? <p className="mt-2 text-[11px] text-muted-foreground">Reported execution evidence. Recorded time is the server receipt time.</p> : null}
      <Button variant="ghost" size="sm" className="mt-2" disabled={replyDisabled} onClick={onReply}>Reply to #{comment.id}</Button>
    </Card>
  );
}

function ReplyReference({ task, commentId, known }: { task: Task; commentId: number; known?: TaskComment }) {
  const [open, setOpen] = useState(false);
  const parentQuery = useQuery({
    queryKey: queryKeys.comment(task.project_id, task.id, commentId),
    queryFn: () => api.comment(task.project_id, task.id, commentId),
    enabled: open && !known,
  });
  const parent = known ?? parentQuery.data;
  return (
    <div className="mt-2 rounded-md border bg-muted/30 p-2">
      <Button variant="ghost" size="sm" className="h-auto p-0 text-xs" onClick={() => setOpen((value) => !value)} aria-expanded={open}>
        {open ? 'Hide' : 'View'} replied comment #{commentId}
      </Button>
      {open ? (
        <div className="mt-2 space-y-2">
          {parentQuery.isError && !parent ? <div role="alert" className="text-xs text-destructive">Could not load comment. <Button variant="outline" size="sm" onClick={() => { void parentQuery.refetch(); }}>Retry</Button></div> : null}
          {!parent && parentQuery.isPending ? <p className="text-xs text-muted-foreground">Loading comment…</p> : null}
          {parent ? <><p className="text-xs text-muted-foreground">Version {parent.task_revision} · {parent.task_revision === task.revision ? 'current' : parent.task_revision < task.revision ? 'older' : 'refresh pending'} · {formatDate(parent.recorded_at)}</p>{parent.execution_result ? <OutcomeBadge outcome={parent.execution_result.outcome} /> : null}<Markdown text={parent.body} /><CommentOutputs comment={parent} /></> : null}
        </div>
      ) : null}
    </div>
  );
}

function CommentOutputs({ comment }: { comment: TaskComment }) {
  const ids = comment.execution_result?.output_source_ids ?? [];
  return ids.length ? (
    <div className="mt-3 space-y-2">
      {ids.map((sourceId) => <SourceCard key={sourceId} projectId={comment.project_id} sourceId={sourceId} label={`Output source #${sourceId}`} />)}
    </div>
  ) : null;
}

function OutcomeBadge({ outcome }: { outcome: Outcome }) {
  const color = outcome === 'success' ? 'border-emerald-300 bg-emerald-50 text-emerald-800' : outcome === 'no_change' ? 'border-sky-300 bg-sky-50 text-sky-800' : 'border-rose-300 bg-rose-50 text-rose-800';
  return <Badge variant="outline" className={color}>{outcome === 'no_change' ? 'No change' : outcome === 'success' ? 'Success' : 'Failure'}</Badge>;
}
