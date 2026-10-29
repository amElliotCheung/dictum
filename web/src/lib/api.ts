import type {
  DecisionList,
  ProjectList,
  RecordTaskComment,
  Source,
  SourceList,
  TaskContext,
  TaskList,
  TaskComment,
  TaskCommentList,
} from './types';

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: { Accept: 'application/json', ...init?.headers },
    cache: 'no-store',
  });
  let payload: unknown = null;
  try {
    payload = await response.json();
  } catch {
    // The status still tells us what happened when the body is not JSON.
  }
  if (!response.ok) {
    const message =
      payload !== null &&
      typeof payload === 'object' &&
      'error' in payload &&
      typeof (payload as { error: unknown }).error === 'string'
        ? (payload as { error: string }).error
        : `Request failed (${response.status})`;
    throw new ApiError(response.status, message);
  }
  return payload as T;
}

function getJson<T>(path: string): Promise<T> {
  return requestJson<T>(path);
}

function projectPath(projectId: string): string {
  return `/api/projects/${encodeURIComponent(projectId)}`;
}

export const api = {
  projects(): Promise<ProjectList> {
    return getJson<ProjectList>('/api/projects');
  },
  tasks(projectId: string, parentTaskId?: number | null): Promise<TaskList> {
    const query = parentTaskId === undefined
      ? ''
      : `?parent_task_id=${parentTaskId === null ? 'null' : parentTaskId}`;
    return getJson<TaskList>(`${projectPath(projectId)}/tasks${query}`);
  },
  decisions(projectId: string): Promise<DecisionList> {
    return getJson<DecisionList>(`${projectPath(projectId)}/decisions`);
  },
  sources(projectId: string): Promise<SourceList> {
    return getJson<SourceList>(`${projectPath(projectId)}/sources`);
  },
  context(projectId: string, taskId: number): Promise<TaskContext> {
    return getJson<TaskContext>(
      `${projectPath(projectId)}/tasks/${taskId}`,
    );
  },
  comments(
    projectId: string,
    taskId: number,
    beforeId?: number,
    hasExecutionResult?: boolean,
  ): Promise<TaskCommentList> {
    const params = new URLSearchParams({ page_size: '20' });
    if (beforeId !== undefined) {
      params.set('before_id', String(beforeId));
    }
    if (hasExecutionResult !== undefined) {
      params.set('has_execution_result', String(hasExecutionResult));
    }
    return getJson<TaskCommentList>(
      `${projectPath(projectId)}/tasks/${taskId}/comments?${params.toString()}`,
    );
  },
  comment(projectId: string, taskId: number, commentId: number): Promise<TaskComment> {
    return getJson<TaskComment>(`${projectPath(projectId)}/tasks/${taskId}/comments/${commentId}`);
  },
  async recordComment(projectId: string, taskId: number, body: RecordTaskComment): Promise<TaskComment> {
    const { token } = await getJson<{ token: string }>('/api/request-token');
    return requestJson<TaskComment>(`${projectPath(projectId)}/tasks/${taskId}/comments`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Dictum-CSRF': token },
      body: JSON.stringify(body),
    });
  },
  source(projectId: string, sourceId: number): Promise<Source> {
    return getJson<Source>(`${projectPath(projectId)}/sources/${sourceId}`);
  },
};

export const queryKeys = {
  projects: ['projects'] as const,
  tasks: (projectId: string | null) =>
    ['projects', projectId, 'tasks'] as const,
  context: (projectId: string, taskId: number) =>
    ['projects', projectId, 'tasks', taskId, 'context'] as const,
  children: (projectId: string, taskId: number) =>
    ['projects', projectId, 'tasks', 'children', taskId] as const,
  comments: (projectId: string, taskId: number) =>
    ['projects', projectId, 'tasks', taskId, 'comments'] as const,
  comment: (projectId: string, taskId: number, commentId: number) =>
    ['projects', projectId, 'tasks', taskId, 'comment', commentId] as const,
  decisions: (projectId: string) =>
    ['projects', projectId, 'decisions'] as const,
  projectSources: (projectId: string) =>
    ['projects', projectId, 'project-sources'] as const,
  source: (projectId: string, sourceId: number) =>
    ['projects', projectId, 'sources', sourceId] as const,
};
