import type { TaskKind, TaskStatus } from './types';

export const statusLabels: Record<TaskStatus, string> = {
  todo: 'Todo',
  in_progress: 'In progress',
  blocked: 'Blocked',
  done: 'Done',
  active: 'Active',
  paused: 'Paused',
};

export function formatKind(kind: TaskKind): string {
  return kind === 'recurring' ? 'Recurring' : 'One-time';
}

export function formatDate(value: string | null | undefined): string {
  if (!value) {
    return '—';
  }
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : date.toLocaleString(undefined, {
        dateStyle: 'medium',
        timeStyle: 'short',
      });
}

export function formatTime(value: number): string {
  if (!value) {
    return '—';
  }
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? '—'
    : date.toLocaleTimeString(undefined, {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
      });
}

export function formatIds(ids: number[]): string {
  return ids.map((id) => `#${id}`).join(', ');
}
