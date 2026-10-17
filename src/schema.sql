CREATE TABLE projects (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND substr(id, 15, 1) = '7' AND substr(id, 20, 1) IN ('8','9','a','b')),
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    description TEXT NOT NULL CHECK (length(trim(description)) > 0),
    created_at TEXT NOT NULL
);

CREATE TABLE tasks (
    id INTEGER PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
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
    updated_at TEXT NOT NULL,
    UNIQUE (project_id, id)
);

CREATE TABLE sources (
    id INTEGER PRIMARY KEY,
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
    id INTEGER PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    statement TEXT NOT NULL CHECK (length(trim(statement)) > 0),
    rationale TEXT NOT NULL CHECK (length(trim(rationale)) > 0),
    supersedes_id INTEGER UNIQUE,
    created_at TEXT NOT NULL,
    UNIQUE (project_id, id),
    FOREIGN KEY (project_id, supersedes_id) REFERENCES decisions(project_id, id),
    CHECK (supersedes_id IS NULL OR supersedes_id != id)
);

CREATE TABLE task_sources (
    project_id TEXT NOT NULL REFERENCES projects(id),
    task_id INTEGER NOT NULL,
    source_id INTEGER NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('context', 'output')),
    FOREIGN KEY (project_id, task_id) REFERENCES tasks(project_id, id),
    FOREIGN KEY (project_id, source_id) REFERENCES sources(project_id, id),
    PRIMARY KEY (task_id, source_id, role)
);

CREATE TABLE task_dependencies (
    project_id TEXT NOT NULL REFERENCES projects(id),
    task_id INTEGER NOT NULL,
    depends_on_task_id INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('before_start', 'before_done')),
    FOREIGN KEY (project_id, task_id) REFERENCES tasks(project_id, id),
    FOREIGN KEY (project_id, depends_on_task_id) REFERENCES tasks(project_id, id),
    PRIMARY KEY (task_id, depends_on_task_id, kind),
    CHECK (task_id != depends_on_task_id)
);

CREATE INDEX tasks_status ON tasks(project_id, status);
CREATE INDEX task_sources_source ON task_sources(source_id);
CREATE INDEX task_dependencies_target ON task_dependencies(depends_on_task_id);

CREATE INDEX sources_project ON sources(project_id);
CREATE INDEX decisions_project ON decisions(project_id);
CREATE INDEX projects_name ON projects(name);
