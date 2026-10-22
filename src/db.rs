use std::path::Path;

use rusqlite::Connection;

// GitHub 仓库信息，字段对应 REST API GET /repos/{owner}/{repo}
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS github_repos (
    id                INTEGER PRIMARY KEY,
    full_name         TEXT    NOT NULL UNIQUE,
    owner             TEXT    NOT NULL,
    name              TEXT    NOT NULL,
    html_url          TEXT    NOT NULL,
    description       TEXT,
    language          TEXT,
    stargazers_count  INTEGER NOT NULL DEFAULT 0,
    forks_count       INTEGER NOT NULL DEFAULT 0,
    open_issues_count INTEGER NOT NULL DEFAULT 0,
    default_branch    TEXT,
    topics            TEXT,
    license           TEXT,
    homepage          TEXT,
    clone_url         TEXT,
    archived          INTEGER NOT NULL DEFAULT 0,
    private           INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT,
    updated_at        TEXT,
    pushed_at         TEXT,
    fetched_at        TEXT    NOT NULL DEFAULT (datetime('now'))
);
"#;

pub fn init(path: &Path) -> rusqlite::Result<()> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_creates_table_and_is_idempotent() {
        let path = std::env::temp_dir().join(format!("startai-db-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        init(&path).unwrap();
        init(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'github_repos'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        drop(conn);
        std::fs::remove_file(&path).unwrap();
    }
}