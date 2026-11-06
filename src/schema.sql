CREATE TABLE projects (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND substr(id, 9, 1) = '-' AND substr(id, 14, 1) = '-' AND substr(id, 19, 1) = '-' AND substr(id, 24, 1) = '-' AND length(replace(id, '-', '')) = 32 AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*' AND substr(id, 15, 1) = '7' AND substr(id, 20, 1) IN ('8','9','a','b')),
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    description TEXT NOT NULL CHECK (length(trim(description)) > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE tasks (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND substr(id, 9, 1) = '-' AND substr(id, 14, 1) = '-' AND substr(id, 19, 1) = '-' AND substr(id, 24, 1) = '-' AND length(replace(id, '-', '')) = 32 AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*' AND substr(id, 15, 1) = '7' AND substr(id, 20, 1) IN ('8','9','a','b')),
    project_id TEXT NOT NULL REFERENCES projects(id),
    parent_task_id TEXT,
    kind TEXT NOT NULL DEFAULT 'one_time' CHECK (kind IN ('one_time', 'recurring')),
    priority TEXT NOT NULL DEFAULT 'medium' CHECK (priority IN ('low', 'medium', 'high')),
    trigger_event TEXT CHECK (trigger_event IS NULL OR (kind = 'recurring' AND length(trim(trigger_event)) > 0)),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    problem_or_goal TEXT NOT NULL CHECK (length(trim(problem_or_goal)) > 0),
    design TEXT NOT NULL CHECK (length(trim(design)) > 0),
    acceptance_criteria TEXT NOT NULL CHECK (json_valid(acceptance_criteria)),
    alternatives_rejected TEXT,
    status TEXT NOT NULL CHECK ((kind = 'one_time' AND status IN ('todo', 'in_progress', 'blocked', 'done')) OR (kind = 'recurring' AND status IN ('active', 'paused'))),
    progress TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (project_id, id),
    FOREIGN KEY (project_id, parent_task_id) REFERENCES tasks(project_id, id),
    CHECK (parent_task_id IS NULL OR parent_task_id != id)
);

CREATE TABLE topics (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND substr(id, 9, 1) = '-' AND substr(id, 14, 1) = '-' AND substr(id, 19, 1) = '-' AND substr(id, 24, 1) = '-' AND length(replace(id, '-', '')) = 32 AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*' AND substr(id, 15, 1) = '7' AND substr(id, 20, 1) IN ('8','9','a','b')),
    project_id TEXT NOT NULL REFERENCES projects(id),
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    name_key TEXT NOT NULL CHECK (length(trim(name_key)) > 0),
    UNIQUE (project_id, id),
    UNIQUE (project_id, name_key)
);

CREATE TABLE sources (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND substr(id, 9, 1) = '-' AND substr(id, 14, 1) = '-' AND substr(id, 19, 1) = '-' AND substr(id, 24, 1) = '-' AND length(replace(id, '-', '')) = 32 AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*' AND substr(id, 15, 1) = '7' AND substr(id, 20, 1) IN ('8','9','a','b')),
    project_id TEXT NOT NULL REFERENCES projects(id),
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    description TEXT NOT NULL CHECK (length(trim(description)) > 0),
    content TEXT,
    locator TEXT,
    context_policy TEXT NOT NULL CHECK (context_policy IN ('always', 'when_relevant')),
    action TEXT CHECK (action IS NULL OR json_valid(action)),
    snapshot TEXT CHECK (snapshot IS NULL OR json_valid(snapshot)),
    latest_attempt TEXT CHECK (latest_attempt IS NULL OR json_valid(latest_attempt)),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (project_id, id),
    CHECK ((content IS NOT NULL) != (locator IS NOT NULL))
);

CREATE TABLE decisions (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND substr(id, 9, 1) = '-' AND substr(id, 14, 1) = '-' AND substr(id, 19, 1) = '-' AND substr(id, 24, 1) = '-' AND length(replace(id, '-', '')) = 32 AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*' AND substr(id, 15, 1) = '7' AND substr(id, 20, 1) IN ('8','9','a','b')),
    project_id TEXT NOT NULL REFERENCES projects(id),
    statement TEXT NOT NULL CHECK (length(trim(statement)) > 0),
    rationale TEXT NOT NULL CHECK (length(trim(rationale)) > 0),
    alternatives_rejected TEXT,
    supersedes_id TEXT UNIQUE,
    created_at TEXT NOT NULL,
    UNIQUE (project_id, id),
    FOREIGN KEY (project_id, supersedes_id) REFERENCES decisions(project_id, id),
    CHECK (supersedes_id IS NULL OR supersedes_id != id)
);

CREATE TABLE task_context_sources (
    project_id TEXT NOT NULL REFERENCES projects(id),
    task_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    FOREIGN KEY (project_id, task_id) REFERENCES tasks(project_id, id),
    FOREIGN KEY (project_id, source_id) REFERENCES sources(project_id, id),
    PRIMARY KEY (task_id, source_id)
);

CREATE TABLE task_topics (
    project_id TEXT NOT NULL REFERENCES projects(id),
    task_id TEXT NOT NULL,
    topic_id TEXT NOT NULL,
    FOREIGN KEY (project_id, task_id) REFERENCES tasks(project_id, id),
    FOREIGN KEY (project_id, topic_id) REFERENCES topics(project_id, id),
    PRIMARY KEY (task_id, topic_id)
);

CREATE TABLE source_topics (
    project_id TEXT NOT NULL REFERENCES projects(id),
    source_id TEXT NOT NULL,
    topic_id TEXT NOT NULL,
    FOREIGN KEY (project_id, source_id) REFERENCES sources(project_id, id),
    FOREIGN KEY (project_id, topic_id) REFERENCES topics(project_id, id),
    PRIMARY KEY (source_id, topic_id)
);

CREATE TABLE decision_topics (
    project_id TEXT NOT NULL REFERENCES projects(id),
    decision_id TEXT NOT NULL,
    topic_id TEXT NOT NULL,
    FOREIGN KEY (project_id, decision_id) REFERENCES decisions(project_id, id),
    FOREIGN KEY (project_id, topic_id) REFERENCES topics(project_id, id),
    PRIMARY KEY (decision_id, topic_id)
);

CREATE INDEX task_topics_topic ON task_topics(project_id, topic_id, task_id);
CREATE INDEX source_topics_topic ON source_topics(project_id, topic_id, source_id);
CREATE INDEX decision_topics_topic ON decision_topics(project_id, topic_id, decision_id);

CREATE TABLE task_dependencies (
    project_id TEXT NOT NULL REFERENCES projects(id),
    task_id TEXT NOT NULL,
    depends_on_task_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('before_start', 'before_done')),
    FOREIGN KEY (project_id, task_id) REFERENCES tasks(project_id, id),
    FOREIGN KEY (project_id, depends_on_task_id) REFERENCES tasks(project_id, id),
    PRIMARY KEY (task_id, depends_on_task_id, kind),
    CHECK (task_id != depends_on_task_id)
);

CREATE TABLE task_comments (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND substr(id, 9, 1) = '-' AND substr(id, 14, 1) = '-' AND substr(id, 19, 1) = '-' AND substr(id, 24, 1) = '-' AND length(replace(id, '-', '')) = 32 AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*' AND substr(id, 15, 1) = '7' AND substr(id, 20, 1) IN ('8','9','a','b')),
    project_id TEXT NOT NULL REFERENCES projects(id),
    task_id TEXT NOT NULL,
    task_revision INTEGER NOT NULL CHECK (task_revision >= 1),
    body TEXT NOT NULL CHECK (length(trim(body)) > 0 AND length(body) <= 4000),
    reply_to_comment_id TEXT,
    outcome TEXT CHECK (outcome IS NULL OR outcome IN ('success', 'no_change', 'failure')),
    recorded_at TEXT NOT NULL,
    UNIQUE (project_id, id),
    UNIQUE (project_id, task_id, id),
    FOREIGN KEY (project_id, task_id) REFERENCES tasks(project_id, id),
    FOREIGN KEY (project_id, task_id, reply_to_comment_id) REFERENCES task_comments(project_id, task_id, id),
    CHECK (reply_to_comment_id IS NULL OR reply_to_comment_id < id)
);

CREATE TABLE task_comment_sources (
    project_id TEXT NOT NULL REFERENCES projects(id),
    comment_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    FOREIGN KEY (project_id, comment_id) REFERENCES task_comments(project_id, id),
    FOREIGN KEY (project_id, source_id) REFERENCES sources(project_id, id),
    PRIMARY KEY (comment_id, source_id)
);

CREATE INDEX tasks_status ON tasks(project_id, status);
CREATE INDEX tasks_kind ON tasks(project_id, kind);
CREATE INDEX tasks_parent ON tasks(project_id, parent_task_id);
CREATE INDEX task_comments_task ON task_comments(project_id, task_id, id);
CREATE INDEX task_comments_execution ON task_comments(project_id, task_id, id) WHERE outcome IS NOT NULL;
CREATE INDEX task_comments_success ON task_comments(project_id, task_id, id) WHERE outcome IN ('success', 'no_change');
CREATE INDEX task_comment_sources_source ON task_comment_sources(source_id);
CREATE INDEX task_context_sources_source ON task_context_sources(source_id);
CREATE INDEX task_dependencies_target ON task_dependencies(depends_on_task_id);

CREATE INDEX sources_project ON sources(project_id);
CREATE INDEX decisions_project ON decisions(project_id);
CREATE INDEX projects_name ON projects(name);
