export interface Topic {
  id: string;
  project_id: string;
  name: string;
}

export interface Project {
  topics: Topic[];
  id: string;
  name: string;
  description: string;
  promoted_decision_ids: string[];
  revision: number;
  created_at: string;
  updated_at: string;
}

export interface ProjectList {
  projects: Project[];
}

export interface PromotedDecision {
  reference_id: string;
  id: string;
  statement: string;
}

export interface ContextProject extends Project {
  promoted_decisions: PromotedDecision[];
}

export type TaskKind = 'one_time' | 'recurring';
export type TaskPriority = 'low' | 'medium' | 'high';

export type TaskStatus =
  | 'todo'
  | 'in_progress'
  | 'blocked'
  | 'done'
  | 'active'
  | 'paused';

export interface DependencyState {
  ready_to_start: boolean;
  unmet_prerequisite_task_ids: string[];
  unmet_completion_task_ids: string[];
}

export interface TaskSummary {
  topics: Topic[];
  id: string;
  project_id: string;
  kind: TaskKind;
  priority: TaskPriority;
  trigger_event: string | null;
  title: string;
  status: TaskStatus;
  progress: string | null;
  dependency_state: DependencyState;
  revision: number;
  updated_at: string;
}

export interface Task {
  topics: Topic[];
  id: string;
  project_id: string;
  parent_task_id: string | null;
  kind: TaskKind;
  priority: TaskPriority;
  trigger_event: string | null;
  title: string;
  problem_or_goal: string;
  design: string;
  acceptance_criteria: string[];
  alternatives_rejected: string | null;
  status: TaskStatus;
  progress: string | null;
  context_source_ids: string[];
  prerequisite_task_ids: string[];
  completion_task_ids: string[];
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
  topics: Topic[];
  id: string;
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

export interface SourceDescriptor {
  topics: Topic[];
  id: string;
  project_id: string;
  name: string;
  description: string;
  representation: 'inline' | 'external';
  context_policy: ContextPolicy;
  has_action: boolean;
  action_due: boolean;
  next_action_at: string | null;
  revision: number;
}

export interface SourceList {
  sources: SourceDescriptor[];
  next_before_id: string | null;
}

export interface Decision {
  topics: Topic[];
  id: string;
  project_id: string;
  statement: string;
  rationale: string;
  alternatives_rejected: string | null;
  supersedes_id: string | null;
  created_at: string;
  is_current: boolean;
  current_decision_id: string;
}

export interface DecisionList {
  decisions: Decision[];
  next_before_id: string | null;
}

export type Outcome = 'success' | 'no_change' | 'failure';

export interface TaskComment {
  id: string;
  project_id: string;
  task_id: string;
  task_revision: number;
  body: string;
  reply_to_comment_id: string | null;
  recorded_at: string;
  execution_result: ExecutionResult | null;
}

export interface ExecutionResult {
  outcome: Outcome;
  output_source_ids: string[];
}

export interface TaskCommentList {
  comments: TaskComment[];
  next_before_id: string | null;
}

export interface RecordTaskComment {
  task_revision: number;
  body: string;
  reply_to_comment_id?: string | null;
  execution_result?: ExecutionResult | null;
}

export interface Context {
  project: ContextProject;
  task: Task | null;
  parent_task: Task | null;
  dependency_state: DependencyState | null;
  current_decisions: Decision[];
  sources: Source[];
  recent_comments: TaskComment[];
  latest_execution: TaskComment | null;
  latest_successful_execution: TaskComment | null;
}

export interface TaskContext extends Context {
  task: Task;
  dependency_state: DependencyState;
}
