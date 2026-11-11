use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Serialize;
use tauri::AppHandle;

use crate::catalog::{Method, Recipe};
use crate::db::{self, InstalledApp};
use crate::jobs::{self, JobLogger, JobManager};
use crate::methods::{self, MethodContext};
use crate::probe;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub glyph: String,
    pub color: String,
    pub icon_url: Option<String>,
    pub draft: bool,
    pub state: String,
    pub version: Option<String>,
    pub managed: bool,
    pub source_url: Option<String>,
    pub method_id: Option<String>,
    pub method_kind: Option<String>,
    /// Newest published version when it is newer than the installed one, taken
    /// from the cached update status so listing stays offline.
    pub update_available: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppDetail {
    pub id: String,
    pub name: String,
    pub description: String,
    pub glyph: String,
    pub color: String,
    pub draft: bool,
    pub state: String,
    pub version: Option<String>,
    pub install_dir: Option<String>,
    pub method: Option<String>,
    pub method_id: Option<String>,
    pub method_kind: Option<String>,
    pub managed: bool,
    pub health_ok: bool,
    pub health_monitored: bool,
    pub source_url: Option<String>,
    pub provenance_json: Option<String>,
    pub bom_path: Option<String>,
    pub usage: Option<crate::catalog::Usage>,
}

pub fn list_apps(
    db_path: &Path,
    app_data: &Path,
    jobs: &JobManager,
) -> Result<Vec<AppSummary>, String> {
    list_apps_with(db_path, app_data, jobs, |recipe, record| {
        let Some(method) = method_for(recipe, record) else {
            return (false, None);
        };
        let health = methods::has_health(method).then(|| methods::health(method));
        (methods::is_running(method, record, app_data), health)
    })
}

fn list_apps_with(
    db_path: &Path,
    app_data: &Path,
    jobs: &JobManager,
    probe_fn: impl Fn(&Recipe, &InstalledApp) -> (bool, Option<bool>),
) -> Result<Vec<AppSummary>, String> {
    let entries = crate::catalog::entries(app_data)?;
    let installed: HashMap<String, InstalledApp> = db::list_installed(db_path)
        .map_err(|err| format!("Failed to load installed applications: {err}"))?
        .into_iter()
        .map(|app| (app.app_id.clone(), app))
        .collect();
    // One query for every cached status, so the grid can show an update badge
    // without a request per application.
    let latest_versions: HashMap<String, String> = db::settings_with_prefix(db_path, "update:")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(app_id, raw)| {
            let status: crate::update::UpdateStatus = serde_json::from_str(&raw).ok()?;
            Some((app_id, status.latest_version))
        })
        .collect();

    Ok(entries
        .into_iter()
        .map(|entry| {
            let recipe = entry.recipe;
            let source_url = entry.source.map(|source| source.url);
            let record = installed.get(&recipe.id);
            let (state, version, managed, method_id, method_kind) =
                match jobs.phase(&recipe.id) {
                    Some(phase) => (
                        phase.state().to_string(),
                        record.map(|app| app.version.clone()),
                        record.map(|app| app.managed).unwrap_or(true),
                        record.and_then(|app| app.method_id.clone()),
                        record.and_then(|app| app.method_kind.clone()),
                    ),
                    None => match record {
                        Some(app) => {
                            let (running, healthy) = probe_fn(&recipe, app);
                            let (state, managed) = reconcile(Some(app), running, healthy);
                            if state != app.state || managed != app.managed {
                                let _ = db::update_installed_state(
                                    db_path,
                                    &recipe.id,
                                    &state,
                                    managed,
                                );
                            }
                            (
                                state,
                                Some(app.version.clone()),
                                managed,
                                app.method_id.clone(),
                                app.method_kind.clone(),
                            )
                        }
                        None => ("not_installed".to_string(), None, true, None, None),
                    },
                };
            let icon_url = recipe.icon_url(source_url.as_deref());
            let draft = recipe.draft;
            // Compared against the installed version rather than the cached
            // `available` flag, so the badge disappears as soon as the upgrade
            // lands and the recorded version catches up.
            let update_available = latest_versions
                .get(&recipe.id)
                .filter(|latest| {
                    version.as_deref().is_some_and(|installed| {
                        crate::update::compare_versions(installed, latest) == Ordering::Less
                    })
                })
                .cloned();
            AppSummary {
                id: recipe.id,
                name: recipe.name,
                description: recipe.description,
                glyph: recipe.glyph,
                color: recipe.color,
                icon_url,
                draft,
                state,
                version,
                managed,
                source_url,
                method_id,
                method_kind,
                update_available,
            }
        })
        .collect())
}

pub fn get_app_detail(
    db_path: &Path,
    app_data: &Path,
    jobs: &JobManager,
    app_id: &str,
) -> Result<AppDetail, String> {
    let entry = crate::catalog::find(app_data, app_id)?;
    let source_url = entry.source.map(|source| source.url);
    let recipe = entry.recipe;
    let record = db::get_installed(db_path, app_id)
        .map_err(|err| format!("Failed to load application state: {err}"))?;
    let method = record
        .as_ref()
        .and_then(|record| method_for(&recipe, record));
    let usage = method
        .and_then(|method| recipe.effective_usage(method))
        .or_else(|| recipe.usage.clone());

    let health_monitored = method.is_some_and(methods::has_health);
    let healthy = health_monitored.then(|| method.is_some_and(methods::health));
    let running = record
        .as_ref()
        .zip(method)
        .map(|(record, method)| methods::is_running(method, record, app_data))
        .unwrap_or(false);
    let (state, managed) = match (&record, jobs.phase(app_id)) {
        (_, Some(phase)) => (phase.state().to_string(), true),
        (Some(app), None) => reconcile(Some(app), running, healthy),
        (None, None) => ("not_installed".to_string(), true),
    };

    if let Some(app) = &record {
        if state != app.state || managed != app.managed {
            let _ = db::update_installed_state(db_path, app_id, &state, managed);
        }
    }

    Ok(AppDetail {
        id: recipe.id,
        name: recipe.name,
        description: recipe.description,
        glyph: recipe.glyph,
        color: recipe.color,
        draft: recipe.draft,
        state,
        version: record.as_ref().map(|app| app.version.clone()),
        install_dir: record.as_ref().map(|app| app.install_dir.clone()),
        method: record.as_ref().map(|app| app.method.clone()),
        method_id: record.as_ref().and_then(|app| app.method_id.clone()),
        method_kind: record.as_ref().and_then(|app| app.method_kind.clone()),
        managed,
        health_ok: healthy.unwrap_or(false),
        health_monitored,
        source_url,
        provenance_json: record.as_ref().and_then(|app| app.provenance_json.clone()),
        bom_path: record.as_ref().and_then(|app| app.bom_path.clone()),
        usage,
    })
}

pub fn reconcile(
    record: Option<&InstalledApp>,
    running: bool,
    health_ok: Option<bool>,
) -> (String, bool) {
    match record {
        None => ("not_installed".to_string(), true),
        Some(_) if running => match health_ok {
            Some(false) => ("failed".to_string(), true),
            _ => ("running".to_string(), true),
        },
        Some(_) if health_ok == Some(true) => ("running".to_string(), false),
        Some(_) => ("stopped".to_string(), true),
    }
}

pub fn refresh_states(
    app: &AppHandle,
    db_path: &Path,
    app_data: &Path,
    jobs: &JobManager,
) -> Result<(), String> {
    let installed = db::list_installed(db_path)
        .map_err(|err| format!("Failed to load installed applications: {err}"))?;
    let entries = crate::catalog::entries(app_data)?;

    for record in installed {
        if jobs.is_active(&record.app_id) {
            continue;
        }
        let Some(recipe) = entries.iter().find(|entry| entry.recipe.id == record.app_id) else {
            continue;
        };
        let Some(method) = method_for(&recipe.recipe, &record) else {
            continue;
        };
        let health = methods::has_health(method).then(|| methods::health(method));
        let (state, managed) = reconcile(
            Some(&record),
            methods::is_running(method, &record, app_data),
            health,
        );
        if state != record.state || managed != record.managed {
            db::update_installed_state(db_path, &record.app_id, &state, managed)
                .map_err(|err| format!("Failed to update application state: {err}"))?;
            jobs::emit_state(app, &record.app_id, &state, Some(&record.version), managed);
        }
    }

    Ok(())
}

pub fn install(
    app: &AppHandle,
    db_path: &Path,
    app_data: &Path,
    install_root: Option<&Path>,
    recipe: &Recipe,
    method: &Method,
    log: &mut JobLogger,
) -> Result<(), String> {
    let platform = probe::current_platform_id()?;
    let context = MethodContext {
        recipe,
        method,
        app_data,
        install_root,
        platform: &platform,
    };
    match methods::install(&context, log) {
        Ok(record) => {
            db::upsert_installed(db_path, &record)
                .map_err(|err| format!("Failed to record the installation: {err}"))?;
            jobs::emit_state(app, &recipe.id, "stopped", Some(&record.version), true);
            log.log(format!("Installed {} {}.", recipe.name, record.version));
            Ok(())
        }
        Err(err) => {
            let _ = fs::remove_dir_all(probe::app_dir(app_data, install_root, &recipe.id));
            Err(err)
        }
    }
}

pub fn start(
    app: &AppHandle,
    db_path: &Path,
    app_data: &Path,
    install_root: Option<&Path>,
    recipe: &Recipe,
    method: &Method,
    log: &mut JobLogger,
) -> Result<(), String> {
    let record = db::get_installed(db_path, &recipe.id)
        .map_err(|err| format!("Failed to load application state: {err}"))?
        .ok_or_else(|| format!("{} is not installed.", recipe.name))?;
    db::update_installed_state(db_path, &recipe.id, "starting", true)
        .map_err(|err| format!("Failed to update application state: {err}"))?;
    jobs::emit_state(app, &recipe.id, "starting", Some(&record.version), true);

    let platform = probe::current_platform_id()?;
    let context = MethodContext {
        recipe,
        method,
        app_data,
        install_root,
        platform: &platform,
    };
    match methods::start(&context, &record, log) {
        Ok(()) => {
            db::update_installed_state(db_path, &recipe.id, "running", true)
                .map_err(|err| format!("Failed to update application state: {err}"))?;
            jobs::emit_state(app, &recipe.id, "running", Some(&record.version), true);
            if method.service.is_some() {
                log.log("Health check passed.");
            } else {
                log.log("Started without health monitoring.");
            }
            Ok(())
        }
        Err(err) => {
            let _ = db::update_installed_state(db_path, &recipe.id, "failed", true);
            jobs::emit_state(app, &recipe.id, "failed", Some(&record.version), true);
            Err(err)
        }
    }
}

pub fn stop(
    app: &AppHandle,
    db_path: &Path,
    app_data: &Path,
    install_root: Option<&Path>,
    recipe: &Recipe,
    method: &Method,
    log: &mut JobLogger,
) -> Result<(), String> {
    let record = db::get_installed(db_path, &recipe.id)
        .map_err(|err| format!("Failed to load application state: {err}"))?
        .ok_or_else(|| format!("{} is not installed.", recipe.name))?;
    db::update_installed_state(db_path, &recipe.id, "stopping", true)
        .map_err(|err| format!("Failed to update application state: {err}"))?;
    jobs::emit_state(app, &recipe.id, "stopping", Some(&record.version), true);

    let platform = probe::current_platform_id()?;
    let context = MethodContext {
        recipe,
        method,
        app_data,
        install_root,
        platform: &platform,
    };
    match methods::stop(&context, &record, log) {
        Ok(()) => {
            db::update_installed_state(db_path, &recipe.id, "stopped", true)
                .map_err(|err| format!("Failed to update application state: {err}"))?;
            jobs::emit_state(app, &recipe.id, "stopped", Some(&record.version), true);
            Ok(())
        }
        Err(err) => {
            let _ = db::update_installed_state(db_path, &recipe.id, "failed", true);
            jobs::emit_state(app, &recipe.id, "failed", Some(&record.version), true);
            Err(err)
        }
    }
}

pub fn uninstall(
    app: &AppHandle,
    db_path: &Path,
    app_data: &Path,
    install_root: Option<&Path>,
    recipe: &Recipe,
    method: &Method,
    log: &mut JobLogger,
) -> Result<(), String> {
    let record = db::get_installed(db_path, &recipe.id)
        .map_err(|err| format!("Failed to load application state: {err}"))?
        .ok_or_else(|| format!("{} is not installed.", recipe.name))?;
    db::update_installed_state(db_path, &recipe.id, "uninstalling", true)
        .map_err(|err| format!("Failed to update application state: {err}"))?;
    jobs::emit_state(app, &recipe.id, "uninstalling", Some(&record.version), true);

    let platform = probe::current_platform_id()?;
    let context = MethodContext {
        recipe,
        method,
        app_data,
        install_root,
        platform: &platform,
    };
    match methods::uninstall(&context, &record, log) {
        Ok(()) => {
            db::delete_installed(db_path, &recipe.id)
                .map_err(|err| format!("Failed to remove the installed record: {err}"))?;
            jobs::emit_state(app, &recipe.id, "not_installed", None, true);
            log.log(format!("Uninstalled {}.", recipe.name));
            Ok(())
        }
        Err(err) => {
            let _ = db::update_installed_state(db_path, &recipe.id, "failed", true);
            jobs::emit_state(app, &recipe.id, "failed", Some(&record.version), true);
            Err(err)
        }
    }
}

pub fn method_for<'a>(recipe: &'a Recipe, record: &InstalledApp) -> Option<&'a Method> {
    record
        .method_id
        .as_deref()
        .and_then(|method_id| recipe.method(method_id))
        .or_else(|| recipe.default_method())
}

#[cfg(test)]
mod tests {
    use super::{list_apps_with, reconcile};
    use crate::db::{self, InstalledApp};
    use crate::jobs::JobManager;

    fn record() -> InstalledApp {
        InstalledApp {
            app_id: "ollama".to_string(),
            version: "1.0.0".to_string(),
            install_dir: "/tmp/ollama".to_string(),
            method: "release".to_string(),
            state: "stopped".to_string(),
            managed: true,
            method_id: Some("release-linux".to_string()),
            method_kind: Some("release".to_string()),
            provenance_json: None,
            bom_path: None,
        }
    }

    #[test]
    fn reconciliation_covers_every_combination() {
        let app = record();
        assert_eq!(
            reconcile(None, false, None),
            ("not_installed".to_string(), true)
        );
        assert_eq!(
            reconcile(Some(&app), true, Some(true)),
            ("running".to_string(), true)
        );
        assert_eq!(
            reconcile(Some(&app), true, Some(false)),
            ("failed".to_string(), true)
        );
        assert_eq!(
            reconcile(Some(&app), true, None),
            ("running".to_string(), true)
        );
        assert_eq!(
            reconcile(Some(&app), false, Some(true)),
            ("running".to_string(), false)
        );
        assert_eq!(
            reconcile(Some(&app), false, Some(false)),
            ("stopped".to_string(), true)
        );
        assert_eq!(
            reconcile(Some(&app), false, None),
            ("stopped".to_string(), true)
        );
    }

    #[test]
    fn detail_exposes_usage_only_when_the_recipe_declares_it() {
        let path = std::env::temp_dir().join(format!(
            "easyai-lifecycle-usage-test-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        db::init(&path).unwrap();
        let data_dir = std::env::temp_dir().join(format!(
            "easyai-lifecycle-usage-data-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&data_dir);
        let imported = data_dir.join("catalog/imported");
        std::fs::create_dir_all(&imported).unwrap();
        let recipe = serde_json::json!({
            "id": "demo",
            "name": "Demo",
            "platforms": ["linux-x86_64"],
            "methods": [{
                "id": "release",
                "kind": "release",
                "when": { "os": ["linux"], "arch": ["x86_64"] },
                "source": { "repo": "owner/repo", "asset": { "linux-x86_64": "demo.tar.gz" } },
                "version": { "type": "github-release", "repo": "owner/repo" },
                "service": { "command": ["demo"], "health_url": "http://127.0.0.1:9/health" },
                "uninstall": { "keep_paths": [] }
            }]
        });
        std::fs::write(
            imported.join("demo.json"),
            serde_json::to_vec(&serde_json::json!({
                "recipe": recipe,
                "source": {
                    "url": "https://example.com/demo.json",
                    "sha256": "abc",
                    "importedAt": "0"
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let jobs = JobManager::new();
        let ollama = super::get_app_detail(&path, &data_dir, &jobs, "ollama").unwrap();
        assert!(ollama.usage.is_some());
        let demo = super::get_app_detail(&path, &data_dir, &jobs, "demo").unwrap();
        assert!(demo.usage.is_none());

        std::fs::remove_dir_all(&data_dir).unwrap();
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn lists_each_catalog_application_once() {
        let path = std::env::temp_dir().join(format!(
            "easyai-lifecycle-list-test-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        db::init(&path).unwrap();
        let data_dir = std::env::temp_dir().join(format!(
            "easyai-lifecycle-data-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&data_dir);
        std::fs::create_dir_all(&data_dir).unwrap();
        let jobs = JobManager::new();

        let apps = list_apps_with(&path, &data_dir, &jobs, |_, _| (false, None)).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "ollama");
        assert_eq!(apps[0].state, "not_installed");
        assert!(apps[0].version.is_none());

        db::upsert_installed(&path, &record()).unwrap();
        let apps = list_apps_with(&path, &data_dir, &jobs, |_, _| (false, None)).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].state, "stopped");
        assert_eq!(apps[0].version.as_deref(), Some("1.0.0"));
        assert_eq!(apps[0].method_id.as_deref(), Some("release-linux"));

        let mut ghost = record();
        ghost.app_id = "ghost".to_string();
        db::upsert_installed(&path, &ghost).unwrap();
        let apps = list_apps_with(&path, &data_dir, &jobs, |_, _| (false, None)).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "ollama");

        std::fs::remove_dir_all(&data_dir).unwrap();
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn reports_an_update_only_when_the_cached_version_is_newer() {
        let path = std::env::temp_dir().join(format!(
            "easyai-lifecycle-update-test-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        db::init(&path).unwrap();
        let data_dir = std::env::temp_dir().join(format!(
            "easyai-lifecycle-update-data-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&data_dir);
        std::fs::create_dir_all(&data_dir).unwrap();
        let jobs = JobManager::new();
        db::upsert_installed(&path, &record()).unwrap();

        let cache = |latest: &str, available: bool| {
            let status = crate::update::UpdateStatus {
                app_id: "ollama".to_string(),
                installed_version: "1.0.0".to_string(),
                latest_version: latest.to_string(),
                available,
                source: "github-release".to_string(),
                checked_at: "1".to_string(),
            };
            db::set_setting(
                &path,
                "update:ollama",
                &serde_json::to_string(&status).unwrap(),
            )
            .unwrap();
        };

        // No cached check yet.
        let apps = list_apps_with(&path, &data_dir, &jobs, |_, _| (false, None)).unwrap();
        assert!(apps[0].update_available.is_none());

        cache("1.2.0", true);
        let apps = list_apps_with(&path, &data_dir, &jobs, |_, _| (false, None)).unwrap();
        assert_eq!(apps[0].update_available.as_deref(), Some("1.2.0"));

        // A stale cache entry must not raise a badge once the versions match.
        cache("1.0.0", true);
        let apps = list_apps_with(&path, &data_dir, &jobs, |_, _| (false, None)).unwrap();
        assert!(apps[0].update_available.is_none());

        std::fs::remove_dir_all(&data_dir).unwrap();
        std::fs::remove_file(&path).unwrap();
    }
}