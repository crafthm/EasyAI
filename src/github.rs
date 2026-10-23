use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use tauri::State;

use crate::db::{self, RepoInfo};
use crate::AppState;

#[derive(Deserialize)]
struct GithubRepo {
    id: i64,
    full_name: String,
    name: String,
    html_url: String,
    owner: GithubOwner,
    description: Option<String>,
    language: Option<String>,
    stargazers_count: i64,
    forks_count: i64,
    open_issues_count: i64,
    default_branch: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    license: Option<GithubLicense>,
    homepage: Option<String>,
    clone_url: String,
    archived: bool,
    private: bool,
    created_at: Option<String>,
    updated_at: Option<String>,
    pushed_at: Option<String>,
}

#[derive(Deserialize)]
struct GithubOwner {
    login: String,
}

#[derive(Deserialize)]
struct GithubLicense {
    spdx_id: Option<String>,
}

#[tauri::command]
pub async fn add_repo(state: State<'_, AppState>, url: String) -> Result<String, String> {
    let db_path = state.db_path.clone();
    let repos_dir = state.repos_dir.clone();
    tauri::async_runtime::spawn_blocking(move || add_repo_blocking(&db_path, &repos_dir, &url))
        .await
        .map_err(|err| format!("Failed to add repository: {err}"))?
}

fn add_repo_blocking(
    db_path: &Path,
    repos_dir: &Path,
    url: &str,
) -> Result<String, String> {
    let (owner, name) = parse_repo_url(url)?;
    let repo = fetch_repo(&owner, &name)?;

    let destination = repos_dir.join(&owner).join(&name);
    if destination.exists() {
        return Err(format!(
            "This repository is already cloned at {}.",
            destination.display()
        ));
    }
    clone_repo(&repo.clone_url, &destination)?;

    db::upsert_repo(
        db_path,
        &RepoInfo {
            id: repo.id,
            full_name: repo.full_name.clone(),
            owner: repo.owner.login,
            name: repo.name,
            html_url: repo.html_url,
            description: repo.description,
            language: repo.language,
            stargazers_count: repo.stargazers_count,
            forks_count: repo.forks_count,
            open_issues_count: repo.open_issues_count,
            default_branch: repo.default_branch,
            topics: serde_json::to_string(&repo.topics).unwrap_or_else(|_| "[]".into()),
            license: repo.license.and_then(|license| license.spdx_id),
            homepage: repo.homepage,
            clone_url: repo.clone_url,
            archived: repo.archived,
            private: repo.private,
            created_at: repo.created_at,
            updated_at: repo.updated_at,
            pushed_at: repo.pushed_at,
        },
    )
    .map_err(|err| format!("Failed to save repository: {err}"))?;

    Ok(repo.full_name)
}

/// Accepts `https://github.com/{owner}/{repo}` with an optional `.git` suffix
/// or trailing slash, and returns `(owner, repo)`.
fn parse_repo_url(input: &str) -> Result<(String, String), String> {
    let invalid = || {
        "Enter a valid GitHub repository URL, e.g. https://github.com/owner/repo.".to_string()
    };

    let url = reqwest::Url::parse(input.trim()).map_err(|_| invalid())?;
    if url.scheme() != "https" || url.host_str() != Some("github.com") {
        return Err(invalid());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid());
    }

    let segments: Vec<&str> = url
        .path_segments()
        .ok_or_else(invalid)?
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() != 2 {
        return Err(invalid());
    }

    let owner = segments[0];
    let name = segments[1].strip_suffix(".git").unwrap_or(segments[1]);
    if !is_valid_name(owner) || !is_valid_name(name) {
        return Err(invalid());
    }

    Ok((owner.to_string(), name.to_string()))
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn fetch_repo(owner: &str, name: &str) -> Result<GithubRepo, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("StartAI/0.1")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| format!("Failed to create HTTP client: {err}"))?;

    let response = client
        .get(format!("https://api.github.com/repos/{owner}/{name}"))
        .send()
        .map_err(|err| format!("Could not reach GitHub: {err}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            404 => "GitHub repository not found (or it is private).".to_string(),
            403 => "GitHub API rate limit exceeded. Try again later.".to_string(),
            _ => format!("GitHub API returned {status}."),
        });
    }

    response
        .json::<GithubRepo>()
        .map_err(|err| format!("Invalid response from GitHub: {err}"))
}

fn clone_repo(clone_url: &str, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }

    let output = Command::new("git")
        .args(["clone", "--quiet"])
        .arg(clone_url)
        .arg(destination)
        .output()
        .map_err(|err| format!("Failed to run git (is Git installed?): {err}"))?;

    if !output.status.success() {
        return Err(format!(
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_repo_url;

    #[test]
    fn parses_supported_urls() {
        for url in [
            "https://github.com/owner/repo",
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo/",
            " https://github.com/owner/repo.git/ ",
        ] {
            assert_eq!(
                parse_repo_url(url).unwrap(),
                ("owner".to_string(), "repo".to_string())
            );
        }
    }

    #[test]
    fn rejects_invalid_urls() {
        for url in [
            "",
            "owner/repo",
            "http://github.com/owner/repo",
            "https://gitlab.com/owner/repo",
            "https://github.com/owner",
            "https://github.com/owner/repo/tree/main",
            "https://github.com/owner/repo?tab=readme",
            "https://github.com/../repo",
            "https://github.com/owner/repo;rm",
        ] {
            assert!(parse_repo_url(url).is_err(), "expected {url:?} to fail");
        }
    }
}