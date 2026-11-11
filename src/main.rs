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
    pub jobs: Arc<jobs::JobManager>,
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("easyai.db");
            db::init(&db_path)?;
            db::mark_running_jobs_failed(&db_path, "Interrupted by application exit.")?;

            let jobs = Arc::new(jobs::JobManager::new());
            app.manage(AppState {
                db_path: db_path.clone(),
                app_data_dir: data_dir.clone(),
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
            commands::list_tools,
            commands::check_updates,
            commands::check_all_updates,
            commands::get_update_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}