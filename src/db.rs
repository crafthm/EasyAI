use std::path::Path;

use rusqlite::{params, Connection};
use serde::Serialize;

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

CREATE TABLE IF NOT EXISTS installed_apps (
    app_id       TEXT PRIMARY KEY,
    version      TEXT NOT NULL,
    install_dir  TEXT NOT NULL,
    method       TEXT NOT NULL,
    state        TEXT NOT NULL,
    managed      INTEGER NOT NULL DEFAULT 1,
    installed_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS jobs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id      TEXT NOT NULL,
    kind        TEXT NOT NULL,
    origin      TEXT NOT NULL DEFAULT 'ui',
    state       TEXT NOT NULL,
    error       TEXT,
    log_path    TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at TEXT
);
"#;

#[derive(Clone, Debug)]
pub struct InstalledApp {
    pub app_id: String,
    pub version: String,
    pub install_dir: String,
    pub method: String,
    pub state: String,
    pub managed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRecord {
    pub id: i64,
    pub app_id: String,
    pub kind: String,
    pub origin: String,
    pub state: String,
    pub error: Option<String>,
    pub log_path: String,
    pub created_at: String,
    pub finished_at: Option<String>,
}

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

pub fn get_installed(path: &Path, app_id: &str) -> rusqlite::Result<Option<InstalledApp>> {
    let conn = Connection::open(path)?;
    let mut stmt = conn.prepare(
        "SELECT app_id, version, install_dir, method, state, managed
         FROM installed_apps WHERE app_id = ?1",
    )?;
    let mut rows = stmt.query(params![app_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(InstalledApp {
            app_id: row.get(0)?,
            version: row.get(1)?,
            install_dir: row.get(2)?,
            method: row.get(3)?,
            state: row.get(4)?,
            managed: row.get::<_, i64>(5)? != 0,
        })),
        None => Ok(None),
    }
}

pub fn list_installed(path: &Path) -> rusqlite::Result<Vec<InstalledApp>> {
    let conn = Connection::open(path)?;
    let mut stmt = conn.prepare(
        "SELECT app_id, version, install_dir, method, state, managed
         FROM installed_apps ORDER BY app_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(InstalledApp {
            app_id: row.get(0)?,
            version: row.get(1)?,
            install_dir: row.get(2)?,
            method: row.get(3)?,
            state: row.get(4)?,
            managed: row.get::<_, i64>(5)? != 0,
        })
    })?;
    rows.collect()
}

pub fn upsert_installed(path: &Path, app: &InstalledApp) -> rusqlite::Result<()> {
    let conn = Connection::open(path)?;
    conn.execute(
        r#"
        INSERT INTO installed_apps (app_id, version, install_dir, method, state, managed)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(app_id) DO UPDATE SET
            version = excluded.version,
            install_dir = excluded.install_dir,
            method = excluded.method,
            state = excluded.state,
            managed = excluded.managed,
            updated_at = datetime('now')
        "#,
        params![
            app.app_id,
            app.version,
            app.install_dir,
            app.method,
            app.state,
            app.managed,
        ],
    )?;
    Ok(())
}

pub fn update_installed_state(
    path: &Path,
    app_id: &str,
    state: &str,
    managed: bool,
) -> rusqlite::Result<()> {
    let conn = Connection::open(path)?;
    conn.execute(
        "UPDATE installed_apps
         SET state = ?2, managed = ?3, updated_at = datetime('now')
         WHERE app_id = ?1",
        params![app_id, state, managed],
    )?;
    Ok(())
}

pub fn delete_installed(path: &Path, app_id: &str) -> rusqlite::Result<()> {
    let conn = Connection::open(path)?;
    conn.execute("DELETE FROM installed_apps WHERE app_id = ?1", params![app_id])?;
    Ok(())
}

pub fn create_job(
    path: &Path,
    app_id: &str,
    kind: &str,
    origin: &str,
    log_path: &str,
) -> rusqlite::Result<i64> {
    let conn = Connection::open(path)?;
    conn.execute(
        "INSERT INTO jobs (app_id, kind, origin, state, log_path)
         VALUES (?1, ?2, ?3, 'running', ?4)",
        params![app_id, kind, origin, log_path],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn set_job_log_path(path: &Path, job_id: i64, log_path: &str) -> rusqlite::Result<()> {
    let conn = Connection::open(path)?;
    conn.execute(
        "UPDATE jobs SET log_path = ?2 WHERE id = ?1",
        params![job_id, log_path],
    )?;
    Ok(())
}

pub fn finish_job(
    path: &Path,
    job_id: i64,
    state: &str,
    error: Option<&str>,
) -> rusqlite::Result<()> {
    let conn = Connection::open(path)?;
    conn.execute(
        "UPDATE jobs
         SET state = ?2, error = ?3, finished_at = datetime('now')
         WHERE id = ?1",
        params![job_id, state, error],
    )?;
    Ok(())
}

pub fn mark_running_jobs_failed(path: &Path, error: &str) -> rusqlite::Result<usize> {
    let conn = Connection::open(path)?;
    conn.execute(
        "UPDATE jobs
         SET state = 'failed', error = ?1, finished_at = datetime('now')
         WHERE state = 'running'",
        params![error],
    )
}

pub fn list_jobs(path: &Path, app_id: &str) -> rusqlite::Result<Vec<JobRecord>> {
    let conn = Connection::open(path)?;
    let mut stmt = conn.prepare(
        "SELECT id, app_id, kind, origin, state, error, log_path, created_at, finished_at
         FROM jobs WHERE app_id = ?1 ORDER BY id DESC",
    )?;
    let rows = stmt.query_map(params![app_id], |row| {
        Ok(JobRecord {
            id: row.get(0)?,
            app_id: row.get(1)?,
            kind: row.get(2)?,
            origin: row.get(3)?,
            state: row.get(4)?,
            error: row.get(5)?,
            log_path: row.get(6)?,
            created_at: row.get(7)?,
            finished_at: row.get(8)?,
        })
    })?;
    rows.collect()
}

pub fn get_job(path: &Path, job_id: i64) -> rusqlite::Result<Option<JobRecord>> {
    let conn = Connection::open(path)?;
    let mut stmt = conn.prepare(
        "SELECT id, app_id, kind, origin, state, error, log_path, created_at, finished_at
         FROM jobs WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![job_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(JobRecord {
            id: row.get(0)?,
            app_id: row.get(1)?,
            kind: row.get(2)?,
            origin: row.get(3)?,
            state: row.get(4)?,
            error: row.get(5)?,
            log_path: row.get(6)?,
            created_at: row.get(7)?,
            finished_at: row.get(8)?,
        })),
        None => Ok(None),
    }
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
            std::env::temp_dir().join(format!("easyai-db-upsert-test-{}.db", std::process::id()));
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
        let path = std::env::temp_dir().join(format!("easyai-db-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        init(&path).unwrap();
        init(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master
                 WHERE type = 'table'
                   AND name IN ('github_repos', 'installed_apps', 'jobs')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);

        drop(conn);
        std::fs::remove_file(&path).unwrap();
    }

    fn sample_installed() -> InstalledApp {
        InstalledApp {
            app_id: "ollama".to_string(),
            version: "0.12.0".to_string(),
            install_dir: "/tmp/easyai/ollama/0.12.0".to_string(),
            method: "github-release-binary".to_string(),
            state: "stopped".to_string(),
            managed: true,
        }
    }

    #[test]
    fn installed_app_roundtrip_and_state_update() {
        let path = std::env::temp_dir().join(format!(
            "easyai-db-installed-test-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        init(&path).unwrap();

        upsert_installed(&path, &sample_installed()).unwrap();
        let record = get_installed(&path, "ollama").unwrap().unwrap();
        assert_eq!(record.version, "0.12.0");
        assert!(record.managed);

        update_installed_state(&path, "ollama", "running", false).unwrap();
        let record = get_installed(&path, "ollama").unwrap().unwrap();
        assert_eq!(record.state, "running");
        assert!(!record.managed);

        assert_eq!(list_installed(&path).unwrap().len(), 1);
        delete_installed(&path, "ollama").unwrap();
        assert!(get_installed(&path, "ollama").unwrap().is_none());

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn job_lifecycle_and_stale_cleanup() {
        let path =
            std::env::temp_dir().join(format!("easyai-db-jobs-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        init(&path).unwrap();

        let job_id = create_job(&path, "ollama", "install", "assistant", "").unwrap();
        set_job_log_path(&path, job_id, "/tmp/job.log").unwrap();
        let job = get_job(&path, job_id).unwrap().unwrap();
        assert_eq!(job.state, "running");
        assert_eq!(job.origin, "assistant");

        finish_job(&path, job_id, "succeeded", None).unwrap();
        let job = get_job(&path, job_id).unwrap().unwrap();
        assert_eq!(job.state, "succeeded");
        assert!(job.finished_at.is_some());

        let stale_id = create_job(&path, "ollama", "start", "ui", "").unwrap();
        let changed = mark_running_jobs_failed(&path, "Interrupted by application exit.").unwrap();
        assert_eq!(changed, 1);
        let stale = get_job(&path, stale_id).unwrap().unwrap();
        assert_eq!(stale.state, "failed");
        assert!(stale.error.unwrap().contains("Interrupted"));

        assert_eq!(list_jobs(&path, "ollama").unwrap().len(), 2);
        std::fs::remove_file(&path).unwrap();
    }
}