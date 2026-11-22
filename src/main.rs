#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bundle;
mod catalog;
mod commands;
mod db;
mod github;
mod jobs;
mod lanscan;
mod lifecycle;
mod logging;
mod methods;
mod probe;
mod proxy;
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

/// Number of job records (and their log files) kept across all applications.
const KEEP_JOBS: usize = 200;

/// Drops the oldest job records and the log files they point at.
fn prune_job_logs(db_path: &std::path::Path) {
    match db::prune_jobs(db_path, KEEP_JOBS) {
        Ok(paths) => {
            for path in &paths {
                if !path.is_empty() {
                    let _ = std::fs::remove_file(path);
                }
            }
            if !paths.is_empty() {
                log::info!("Pruned {} old job log(s).", paths.len());
            }
        }
        Err(err) => log::warn!("Failed to prune old job logs: {err}"),
    }
}

fn main() {
    // Resolve the desktop proxy before any HTTP client or WebKit process exists.
    proxy::init();
    if std::env::var_os("EASYAI_DEBUG_PROXY").is_some() {
        eprintln!(
            "proxy debug: HTTPS_PROXY={:?} NO_PROXY={:?}",
            std::env::var("HTTPS_PROXY"),
            std::env::var("NO_PROXY")
        );
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                proxy::disable_webview_proxy(&window);
            }

            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            if let Err(err) = logging::init(&probe::logs_dir(&data_dir)) {
                eprintln!("Failed to initialize logging: {err}");
            }
            log::info!("EasyAI starting (data dir: {})", data_dir.display());
            let db_path = data_dir.join("easyai.db");
            if let Err(err) = db::init(&db_path) {
                log::error!("Failed to open {}: {err}", db_path.display());
                return Err(err.into());
            }
            db::mark_running_jobs_failed(&db_path, "Interrupted by application exit.")?;
            prune_job_logs(&db_path);

            // A bad path must not stop the app from starting: fall back to the
            // default root and let the settings tab surface the problem.
            let install_root = match resolve_install_root(&db_path) {
                Ok(root) => root,
                Err(err) => {
                    log::warn!("{err} Falling back to the default install root.");
                    None
                }
            };
            if let Some(root) = &install_root {
                if let Err(err) = std::fs::create_dir_all(root) {
                    log::warn!("Failed to create {}: {err}", root.display());
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
            if let Err(err) = lifecycle::refresh_states(&handle, &db_path, &data_dir, &jobs) {
                log::warn!("Initial state refresh failed: {err}");
            }

            // Cloned before the state loop takes ownership of the originals.
            let update_handle = app.handle().clone();
            let update_db = db_path.clone();
            let update_data = data_dir.clone();

            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(5));
                if let Err(err) =
                    lifecycle::refresh_states(&handle, &db_path, &data_dir, &jobs)
                {
                    log::warn!("State refresh failed: {err}");
                }
            });

            // Version checks talk to GitHub, so they run on their own thread at
            // most once per `CHECK_INTERVAL_SECS`. The loop itself wakes hourly
            // and only spends requests when a sweep is actually due.
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(15));
                loop {
                    if update::due(&update_db) {
                        update::check_all(&update_handle, &update_db, &update_data);
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
            commands::remove_app,
            commands::save_start_command,
            commands::preview_bundle,
            commands::import_bundle,
            commands::export_bundle,
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
            commands::get_update_status,
            commands::scan_lan,
            commands::log_ui_error
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}