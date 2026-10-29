import { useSyncExternalStore } from 'react';
import type { Outcome } from './types';

export interface CommentDraft {
  body: string;
  taskRevision: number | null;
  replyToCommentId: number | null;
  outcome: Outcome | '';
  outputSourceIds: number[];
  sending: boolean;
  error: string | null;
  notice: string | null;
}

const drafts = new Map<string, CommentDraft>();
const listeners = new Set<() => void>();

function emptyDraft(): CommentDraft {
  return {
    body: '', taskRevision: null, replyToCommentId: null, outcome: '',
    outputSourceIds: [], sending: false, error: null, notice: null,
  };
}

export function draftKey(projectId: string, taskId: number): string {
  return `dictum-comment:${projectId}:${taskId}`;
}

export function readDraft(key: string): CommentDraft {
  const existing = drafts.get(key);
  if (existing) return existing;
  let draft = emptyDraft();
  try {
    const saved: unknown = JSON.parse(sessionStorage.getItem(key) ?? 'null');
    if (saved && typeof saved === 'object' && 'body' in saved && typeof saved.body === 'string') {
      const value = saved as Partial<CommentDraft>;
      draft = {
        ...draft,
        body: value.body ?? '',
        taskRevision: Number.isInteger(value.taskRevision) ? value.taskRevision! : null,
        replyToCommentId: Number.isInteger(value.replyToCommentId) ? value.replyToCommentId! : null,
        outcome: ['success', 'no_change', 'failure'].includes(value.outcome ?? '') ? value.outcome! : '',
        outputSourceIds: Array.isArray(value.outputSourceIds)
          ? value.outputSourceIds.filter((id) => Number.isInteger(id) && id > 0) : [],
        error: value.sending
          ? 'A previous submission may have completed. Check the timeline before posting this draft again.'
          : null,
      };
    }
  } catch { /* Drafts remain usable when session storage is unavailable. */ }
  drafts.set(key, draft);
  return draft;
}

export function updateDraft(key: string, change: Partial<CommentDraft>): void {
  const draft = { ...readDraft(key), ...change };
  drafts.set(key, draft);
  try {
    sessionStorage.setItem(key, JSON.stringify(draft));
  } catch { /* In-memory drafts still survive navigation. */ }
  for (const listener of listeners) listener();
}

export function clearDraft(key: string, notice: string | null = null): void {
  drafts.set(key, { ...emptyDraft(), notice });
  try { sessionStorage.removeItem(key); } catch { /* Storage is optional. */ }
  for (const listener of listeners) listener();
}

export function useCommentDraft(key: string): CommentDraft {
  return useSyncExternalStore(
    (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    () => readDraft(key),
  );
}
