import type {
  DecisionList,
  ProjectList,
  RunAgent,
  RunConfig,
  RunDetail,
  RunList,
  RunSummary,
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

async function requestJson<T>(
  path: string,
  init?: RequestInit,
): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      Accept: 'application/json',
      ...init?.headers,
    },
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

function postJson<T>(path: string, token: string, body: unknown): Promise<T> {
  return requestJson<T>(path, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'X-Dictum-CSRF': token,
    },
    body: JSON.stringify(body),
  });
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
  runConfig(): Promise<RunConfig> {
    return getJson<RunConfig>('/api/run-token');
  },
  runs(projectId: string, taskId: number): Promise<RunList> {
    return getJson<RunList>(`${projectPath(projectId)}/tasks/${taskId}/runs`);
  },
  run(
    projectId: string,
    taskId: number,
    runId: number,
    includeLog = true,
  ): Promise<RunDetail> {
    const query = includeLog ? '' : '?include_log=0';
    return getJson<RunDetail>(
      `${projectPath(projectId)}/tasks/${taskId}/runs/${runId}${query}`,
    );
  },
  launchRun(
    projectId: string,
    taskId: number,
    token: string,
    agent: RunAgent,
    workingDirectory: string,
  ): Promise<RunSummary> {
    return postJson<RunSummary>(
      `${projectPath(projectId)}/tasks/${taskId}/runs`,
      token,
      { agent, working_directory: workingDirectory },
    );
  },
  cancelRun(
    projectId: string,
    taskId: number,
    runId: number,
    token: string,
  ): Promise<RunSummary> {
    return postJson<RunSummary>(
      `${projectPath(projectId)}/tasks/${taskId}/runs/${runId}/cancel`,
      token,
      {},
    );
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
  runConfig: ['run-config'] as const,
  runs: (projectId: string, taskId: number) =>
    ['projects', projectId, 'tasks', taskId, 'runs'] as const,
  run: (projectId: string, taskId: number, runId: number) =>
    ['projects', projectId, 'tasks', taskId, 'runs', runId] as const,
  runLog: (projectId: string, taskId: number, runId: number) =>
    ['projects', projectId, 'tasks', taskId, 'runs', runId, 'log'] as const,
};
