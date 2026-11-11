use std::cmp::Ordering;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::catalog::{self, Method};
use crate::db;
use crate::github;

/// How long to wait between automatic background checks. GitHub's
/// unauthenticated API allows 60 requests per hour, so a daily sweep is safe
/// even with a few dozen installed applications.
pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Upper bound on repositories contacted in a single sweep, so a large catalog
/// cannot exhaust the API quota.
const MAX_CHECKS_PER_RUN: usize = 20;

const LAST_CHECK_KEY: &str = "update_checked_at";
const CACHE_PREFIX: &str = "update:";

/// The result of comparing the installed version against the newest published
/// one. Cached in the settings table so the UI can show it without a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub app_id: String,
    pub installed_version: String,
    pub latest_version: String,
    pub available: bool,
    /// Where the latest version came from, e.g. `github-release`.
    pub source: String,
    /// Epoch seconds of the check that produced this record.
    pub checked_at: String,
}

/// Emitted only when a newer version is actually found, so the UI is not
/// redrawn on every sweep.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEvent {
    pub app_id: String,
    pub installed_version: String,
    pub latest_version: String,
}

/// Compares two version strings, newest last.
///
/// Both sides are split into runs of digits and letters, so `1.10` sorts after
/// `1.9` and `1.2` equals `1.2.0`. A pre-release suffix (`1.0.0-beta`) sorts
/// before the plain release (`1.0.0`).
pub fn compare_versions(left: &str, right: &str) -> Ordering {
    let left = tokenize(left);
    let right = tokenize(right);
    let width = left.len().max(right.len());
    for index in 0..width {
        let ordering = match (left.get(index), right.get(index)) {
            (Some(Token::Num(a)), Some(Token::Num(b))) => a.cmp(b),
            (Some(Token::Num(a)), None) => a.cmp(&0),
            (None, Some(Token::Num(b))) => 0u64.cmp(b),
            (Some(Token::Text(a)), Some(Token::Text(b))) => a.cmp(b),
            // A trailing pre-release marker sorts before the bare release.
            (Some(Token::Text(_)), None) => Ordering::Less,
            (None, Some(Token::Text(_))) => Ordering::Greater,
            // Numbers sort before text at the same position.
            (Some(Token::Num(_)), Some(Token::Text(_))) => Ordering::Less,
            (Some(Token::Text(_)), Some(Token::Num(_))) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Num(u64),
    Text(String),
}

fn tokenize(value: &str) -> Vec<Token> {
    let value = normalize_tag(value);
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut numeric = false;
    for ch in value.chars() {
        if !ch.is_ascii_alphanumeric() {
            flush(&mut tokens, &mut current, numeric);
            continue;
        }
        let is_numeric = ch.is_ascii_digit();
        if !current.is_empty() && is_numeric != numeric {
            flush(&mut tokens, &mut current, numeric);
        }
        numeric = is_numeric;
        current.push(ch);
    }
    flush(&mut tokens, &mut current, numeric);
    tokens
}

fn flush(tokens: &mut Vec<Token>, current: &mut String, numeric: bool) {
    if current.is_empty() {
        return;
    }
    if numeric {
        tokens.push(Token::Num(current.parse().unwrap_or(u64::MAX)));
    } else {
        tokens.push(Token::Text(current.to_ascii_lowercase()));
    }
    current.clear();
}

/// Drops the `v` prefix GitHub tags usually carry.
fn normalize_tag(tag: &str) -> String {
    tag.trim()
        .trim_start_matches(['v', 'V'])
        .trim()
        .to_string()
}

struct Latest {
    version: String,
    source: String,
}

/// Newest version published for the installed method.
///
/// Only `github-release` sources are resolved; other kinds return an
/// explanatory error instead of silently reporting "up to date".
fn latest_version(method: &Method) -> Result<Latest, String> {
    let kind = method
        .version
        .as_ref()
        .map(|source| source.kind.as_str())
        .unwrap_or("github-release");
    if kind != "github-release" {
        return Err(format!(
            "Checking updates for the '{kind}' version source is not supported yet."
        ));
    }
    let repo = method
        .version
        .as_ref()
        .and_then(|source| source.repo.clone())
        .or_else(|| {
            method
                .source
                .as_ref()
                .and_then(|source| source.repo.clone())
        })
        .ok_or_else(|| "The install method has no repository to check.".to_string())?;
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| format!("'{repo}' is not a valid GitHub repository."))?;
    let release = github::latest_release(owner, name)?
        .ok_or_else(|| format!("{repo} has not published a release."))?;
    Ok(Latest {
        version: normalize_tag(&release.tag_name),
        source: "github-release".to_string(),
    })
}

/// Checks one installed application and caches the outcome.
pub fn check(db_path: &Path, app_data: &Path, app_id: &str) -> Result<UpdateStatus, String> {
    let recipe = catalog::find(app_data, app_id)?.recipe;
    let record = db::get_installed(db_path, app_id)
        .map_err(|err| format!("Failed to load application state: {err}"))?
        .ok_or_else(|| "The application is not installed.".to_string())?;
    let method = crate::lifecycle::method_for(&recipe, &record)
        .ok_or_else(|| "The installed method is no longer present in the recipe.".to_string())?;

    let latest = latest_version(method)?;
    let available = compare_versions(&record.version, &latest.version) == Ordering::Less;
    let status = UpdateStatus {
        app_id: record.app_id.clone(),
        installed_version: record.version.clone(),
        latest_version: latest.version,
        available,
        source: latest.source,
        checked_at: now(),
    };
    store(db_path, &status);
    Ok(status)
}

/// The last known result for an application, without any network access.
pub fn cached(db_path: &Path, app_id: &str) -> Option<UpdateStatus> {
    db::get_setting(db_path, &format!("{CACHE_PREFIX}{app_id}"))
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

/// Checks every installed application, emitting an event for each one that has
/// a newer version. Failures are skipped: one unreachable repository must not
/// stop the sweep.
pub fn check_all(app: &AppHandle, db_path: &Path, app_data: &Path) -> Vec<UpdateStatus> {
    let records = db::list_installed(db_path).unwrap_or_default();
    let mut statuses = Vec::new();
    for record in records.into_iter().take(MAX_CHECKS_PER_RUN) {
        let Ok(status) = check(db_path, app_data, &record.app_id) else {
            continue;
        };
        if status.available {
            emit(app, &status);
        }
        statuses.push(status);
    }
    mark_checked(db_path);
    statuses
}

/// Whether the last full sweep is older than the check interval.
pub fn due(db_path: &Path) -> bool {
    let last = db::get_setting(db_path, LAST_CHECK_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    now_secs().saturating_sub(last) >= CHECK_INTERVAL_SECS
}

fn mark_checked(db_path: &Path) {
    let _ = db::set_setting(db_path, LAST_CHECK_KEY, &now());
}

fn store(db_path: &Path, status: &UpdateStatus) {
    if let Ok(json) = serde_json::to_string(status) {
        let _ = db::set_setting(db_path, &format!("{CACHE_PREFIX}{}", status.app_id), &json);
    }
}

fn emit(app: &AppHandle, status: &UpdateStatus) {
    let _ = app.emit(
        "app-update-available",
        UpdateEvent {
            app_id: status.app_id.clone(),
            installed_version: status.installed_version.clone(),
            latest_version: status.latest_version.clone(),
        },
    );
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn now() -> String {
    now_secs().to_string()
}

#[cfg(test)]
mod tests {
    use super::{compare_versions, normalize_tag, tokenize, Token};
    use std::cmp::Ordering;

    #[test]
    fn compares_numeric_components_numerically() {
        assert_eq!(compare_versions("1.9", "1.10"), Ordering::Less);
        assert_eq!(compare_versions("1.10", "1.9"), Ordering::Greater);
        assert_eq!(compare_versions("2.0", "2.0"), Ordering::Equal);
    }

    #[test]
    fn ignores_missing_trailing_zeros() {
        assert_eq!(compare_versions("1.2", "1.2.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.2.0", "1.2"), Ordering::Equal);
        assert_eq!(compare_versions("1.2", "1.2.1"), Ordering::Less);
    }

    #[test]
    fn strips_the_version_prefix() {
        assert_eq!(compare_versions("v0.27.1", "0.27.1"), Ordering::Equal);
        assert_eq!(compare_versions("V1.0", "v1.1"), Ordering::Less);
        assert_eq!(normalize_tag(" v1.2.3 "), "1.2.3");
    }

    #[test]
    fn sorts_prereleases_before_the_release() {
        assert_eq!(compare_versions("1.0.0-beta", "1.0.0"), Ordering::Less);
        assert_eq!(compare_versions("1.0.0", "1.0.0-beta"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0-beta", "1.0.0-alpha"), Ordering::Greater);
    }

    #[test]
    fn tokenizes_mixed_versions() {
        assert_eq!(
            tokenize("1.2.3-rc1"),
            vec![
                Token::Num(1),
                Token::Num(2),
                Token::Num(3),
                Token::Text("rc".to_string()),
                Token::Num(1),
            ]
        );
    }
}
