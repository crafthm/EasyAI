#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod catalog;
mod commands;
mod db;
mod github;
mod jobs;
mod lifecycle;
mod methods;
mod probe;
mod resolve;
mod tools;
mod update;
mod usage;
mod usage_infer;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;

pub struct AppState {
    pub db_path: PathBuf,
    pub app_data_dir: PathBuf,
    /// Resolved install root. `None` means the default `<app_data>/apps`.
    /// Read from settings at startup, so changing it takes effect after a
    /// restart — which is also what keeps already-installed apps reachable.
    pub install_root: Option<PathBuf>,
    pub jobs: Arc<jobs::JobManager>,
}

/// Settings key holding the user-configured install root.
pub const INSTALL_ROOT_KEY: &str = "install_root";

/// Resolve the configured install root.
///
/// Returns `None` when unset or blank, meaning "use `<app_data>/apps`". A
/// relative path is rejected rather than silently joined onto the process
/// working directory, which differs between dev and a bundled app.
pub fn resolve_install_root(db_path: &std::path::Path) -> Result<Option<PathBuf>, String> {
    let raw = db::get_setting(db_path, INSTALL_ROOT_KEY)
        .map_err(|err| format!("Failed to read install root: {err}"))?
        .unwrap_or_default();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(format!(
            "Install root '{}' must be an absolute path.",
            path.display()
        ));
    }
    Ok(Some(path))
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("easyai.db");
            db::init(&db_path)?;
            db::mark_running_jobs_failed(&db_path, "Interrupted by application exit.")?;

            // A bad path must not stop the app from starting: fall back to the
            // default root and let the settings tab surface the problem.
            let install_root = match resolve_install_root(&db_path) {
                Ok(root) => root,
                Err(err) => {
                    eprintln!("{err} Falling back to the default install root.");
                    None
                }
            };
            if let Some(root) = &install_root {
                if let Err(err) = std::fs::create_dir_all(root) {
                    eprintln!("Failed to create {}: {err}", root.display());
                }
            }

            let jobs = Arc::new(jobs::JobManager::new());
            app.manage(AppState {
                db_path: db_path.clone(),
                app_data_dir: data_dir.clone(),
                install_root,
                jobs: Arc::clone(&jobs),
            });

            let handle = app.handle().clone();
            let _ = lifecycle::refresh_states(&handle, &db_path, &data_dir, &jobs);

            // Cloned before the state loop takes ownership of the originals.
            let update_handle = app.handle().clone();
            let update_db = db_path.clone();
            let update_data = data_dir.clone();

            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(5));
                let _ = lifecycle::refresh_states(&handle, &db_path, &data_dir, &jobs);
            });

            // Version checks talk to GitHub, so they run on their own thread at
            // most once per `CHECK_INTERVAL_SECS`. The loop itself wakes hourly
            // and only spends requests when a sweep is actually due.
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(15));
                loop {
                    if update::due(&update_db) {
                        let _ = update::check_all(&update_handle, &update_db, &update_data);
                    }
                    std::thread::sleep(Duration::from_secs(3600));
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            github::add_repo,
            commands::list_apps,
            commands::get_app_detail,
            commands::plan_install,
            commands::install_app,
            commands::start_app,
            commands::stop_app,
            commands::uninstall_app,
            commands::list_jobs,
            commands::get_job_log,
            commands::preview_recipe,
            commands::import_recipe,
            commands::remove_recipe,
            commands::get_bom,
            commands::open_url,
            commands::open_local_url,
            commands::usage_check,
            commands::get_usage_report,
            commands::discover_usage,
            commands::reinfer_usage,
            commands::get_method_settings,
            commands::save_method_settings,
            commands::get_install_root,
            commands::save_install_root,
            commands::list_tools,
            commands::check_updates,
            commands::check_all_updates,
            commands::get_update_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}