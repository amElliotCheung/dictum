export interface Project {
  id: string;
  name: string;
  description: string;
  created_at: string;
}

export interface ProjectList {
  projects: Project[];
}

export type TaskKind = 'one_time' | 'recurring';

export type TaskStatus =
  | 'todo'
  | 'in_progress'
  | 'blocked'
  | 'done'
  | 'active'
  | 'paused';

export interface DependencyState {
  ready_to_start: boolean;
  unmet_prerequisite_task_ids: number[];
  unmet_completion_task_ids: number[];
}

export interface TaskSummary {
  id: number;
  project_id: string;
  kind: TaskKind;
  trigger_event: string | null;
  title: string;
  status: TaskStatus;
  progress: string | null;
  dependency_state: DependencyState;
  revision: number;
  updated_at: string;
}

export interface Task {
  id: number;
  project_id: string;
  kind: TaskKind;
  trigger_event: string | null;
  title: string;
  problem_or_goal: string;
  design: string;
  acceptance_criteria: string[];
  alternatives_rejected: string | null;
  status: TaskStatus;
  progress: string | null;
  context_source_ids: number[];
  prerequisite_task_ids: number[];
  completion_task_ids: number[];
  revision: number;
  created_at: string;
  updated_at: string;
}

export interface TaskList {
  tasks: TaskSummary[];
}

export type ContextPolicy = 'always' | 'when_relevant';

export type ActionTrigger =
  | { kind: 'manual' }
  | { kind: 'interval'; every_minutes: number };

export interface SourceAction {
  instruction: string;
  trigger: ActionTrigger;
}

export interface SourceSnapshot {
  observed_at: string;
  summary: string;
  checkpoint: string | null;
  references: string[];
}

export interface SourceAttempt {
  recorded_at: string;
  outcome: 'success' | 'no_change' | 'failure';
  error: string | null;
}

export interface Source {
  id: number;
  project_id: string;
  name: string;
  description: string;
  content: string | null;
  locator: string | null;
  context_policy: ContextPolicy;
  action: SourceAction | null;
  snapshot: SourceSnapshot | null;
  latest_attempt: SourceAttempt | null;
  action_due: boolean;
  next_action_at: string | null;
  revision: number;
  created_at: string;
  updated_at: string;
}

export interface Decision {
  id: number;
  project_id: string;
  statement: string;
  rationale: string;
  supersedes_id: number | null;
  created_at: string;
  is_current: boolean;
}

export type Outcome = 'success' | 'no_change' | 'failure';

export interface TaskResult {
  id: number;
  project_id: string;
  task_id: number;
  outcome: Outcome;
  evidence: string;
  output_source_ids: number[];
  recorded_at: string;
  task_revision: number;
}

export interface TaskResultList {
  results: TaskResult[];
  next_before_id: number | null;
}

export interface TaskContext {
  task: Task;
  dependency_state: DependencyState;
  current_decisions: Decision[];
  sources: Source[];
  latest_result: TaskResult | null;
  latest_successful_result: TaskResult | null;
}
