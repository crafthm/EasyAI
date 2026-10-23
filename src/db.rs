use std::path::Path;

use rusqlite::{params, Connection};

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

pub struct RepoInfo {
    pub id: i64,
    pub full_name: String,
    pub owner: String,
    pub name: String,
    pub html_url: String,
    pub description: Option<String>,
    pub language: Option<String>,
    pub stargazers_count: i64,
    pub forks_count: i64,
    pub open_issues_count: i64,
    pub default_branch: Option<String>,
    pub topics: String,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub clone_url: String,
    pub archived: bool,
    pub private: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub pushed_at: Option<String>,
}

pub fn upsert_repo(path: &Path, repo: &RepoInfo) -> rusqlite::Result<()> {
    let conn = Connection::open(path)?;
    conn.execute(
        r#"
        INSERT INTO github_repos (
            id, full_name, owner, name, html_url, description, language,
            stargazers_count, forks_count, open_issues_count, default_branch,
            topics, license, homepage, clone_url, archived, private,
            created_at, updated_at, pushed_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
        )
        ON CONFLICT(id) DO UPDATE SET
            full_name = excluded.full_name,
            owner = excluded.owner,
            name = excluded.name,
            html_url = excluded.html_url,
            description = excluded.description,
            language = excluded.language,
            stargazers_count = excluded.stargazers_count,
            forks_count = excluded.forks_count,
            open_issues_count = excluded.open_issues_count,
            default_branch = excluded.default_branch,
            topics = excluded.topics,
            license = excluded.license,
            homepage = excluded.homepage,
            clone_url = excluded.clone_url,
            archived = excluded.archived,
            private = excluded.private,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at,
            pushed_at = excluded.pushed_at,
            fetched_at = datetime('now')
        "#,
        params![
            repo.id,
            repo.full_name,
            repo.owner,
            repo.name,
            repo.html_url,
            repo.description,
            repo.language,
            repo.stargazers_count,
            repo.forks_count,
            repo.open_issues_count,
            repo.default_branch,
            repo.topics,
            repo.license,
            repo.homepage,
            repo.clone_url,
            repo.archived,
            repo.private,
            repo.created_at,
            repo.updated_at,
            repo.pushed_at,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_repo() -> RepoInfo {
        RepoInfo {
            id: 1,
            full_name: "owner/repo".to_string(),
            owner: "owner".to_string(),
            name: "repo".to_string(),
            html_url: "https://github.com/owner/repo".to_string(),
            description: Some("Demo".to_string()),
            language: Some("Rust".to_string()),
            stargazers_count: 1,
            forks_count: 0,
            open_issues_count: 0,
            default_branch: Some("main".to_string()),
            topics: "[]".to_string(),
            license: None,
            homepage: None,
            clone_url: "https://github.com/owner/repo.git".to_string(),
            archived: false,
            private: false,
            created_at: None,
            updated_at: None,
            pushed_at: None,
        }
    }

    #[test]
    fn upsert_repo_inserts_and_updates() {
        let path =
            std::env::temp_dir().join(format!("startai-db-upsert-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        init(&path).unwrap();

        let mut repo = sample_repo();
        upsert_repo(&path, &repo).unwrap();
        repo.stargazers_count = 42;
        upsert_repo(&path, &repo).unwrap();

        let conn = Connection::open(&path).unwrap();
        let (count, stars, full_name): (i64, i64, String) = conn
            .query_row(
                "SELECT count(*), stargazers_count, full_name FROM github_repos",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(stars, 42);
        assert_eq!(full_name, "owner/repo");

        drop(conn);
        std::fs::remove_file(&path).unwrap();
    }

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