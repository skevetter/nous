CREATE TABLE IF NOT EXISTS schema_version (
    id INTEGER PRIMARY KEY,
    version TEXT NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
