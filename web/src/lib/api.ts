import type {
  DecisionList,
  ProjectList,
  Source,
  SourceList,
  TaskContext,
  TaskList,
  TaskResultList,
} from './types';

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

async function getJson<T>(path: string): Promise<T> {
  const response = await fetch(path, {
    headers: { Accept: 'application/json' },
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

function projectPath(projectId: string): string {
  return `/api/projects/${encodeURIComponent(projectId)}`;
}

export const api = {
  projects(): Promise<ProjectList> {
    return getJson<ProjectList>('/api/projects');
  },
  tasks(projectId: string): Promise<TaskList> {
    return getJson<TaskList>(`${projectPath(projectId)}/tasks`);
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
  results(
    projectId: string,
    taskId: number,
    beforeId?: number,
  ): Promise<TaskResultList> {
    const params = new URLSearchParams({ page_size: '20' });
    if (beforeId !== undefined) {
      params.set('before_id', String(beforeId));
    }
    return getJson<TaskResultList>(
      `${projectPath(projectId)}/tasks/${taskId}/results?${params.toString()}`,
    );
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
  results: (projectId: string, taskId: number) =>
    ['projects', projectId, 'tasks', taskId, 'results'] as const,
  decisions: (projectId: string) =>
    ['projects', projectId, 'decisions'] as const,
  projectSources: (projectId: string) =>
    ['projects', projectId, 'project-sources'] as const,
  source: (projectId: string, sourceId: number) =>
    ['projects', projectId, 'sources', sourceId] as const,
};
