CREATE TABLE tasks (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    problem_or_goal TEXT NOT NULL CHECK (length(trim(problem_or_goal)) > 0),
    design TEXT NOT NULL CHECK (length(trim(design)) > 0),
    acceptance_criteria TEXT NOT NULL CHECK (json_valid(acceptance_criteria)),
    alternatives_rejected TEXT,
    status TEXT NOT NULL CHECK (status IN ('todo', 'in_progress', 'blocked', 'done')),
    progress TEXT,
    result TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE sources (
    id INTEGER PRIMARY KEY,
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
    CHECK ((content IS NOT NULL) != (locator IS NOT NULL))
);

CREATE TABLE decisions (
    id INTEGER PRIMARY KEY,
    statement TEXT NOT NULL CHECK (length(trim(statement)) > 0),
    rationale TEXT NOT NULL CHECK (length(trim(rationale)) > 0),
    supersedes_id INTEGER UNIQUE REFERENCES decisions(id),
    created_at TEXT NOT NULL,
    CHECK (supersedes_id IS NULL OR supersedes_id != id)
);

CREATE TABLE task_sources (
    task_id INTEGER NOT NULL REFERENCES tasks(id),
    source_id INTEGER NOT NULL REFERENCES sources(id),
    role TEXT NOT NULL CHECK (role IN ('context', 'output')),
    PRIMARY KEY (task_id, source_id, role)
);

CREATE TABLE task_dependencies (
    task_id INTEGER NOT NULL REFERENCES tasks(id),
    depends_on_task_id INTEGER NOT NULL REFERENCES tasks(id),
    kind TEXT NOT NULL CHECK (kind IN ('before_start', 'before_done')),
    PRIMARY KEY (task_id, depends_on_task_id, kind),
    CHECK (task_id != depends_on_task_id)
);

CREATE INDEX tasks_status ON tasks(status);
CREATE INDEX task_sources_source ON task_sources(source_id);
CREATE INDEX task_dependencies_target ON task_dependencies(depends_on_task_id);
