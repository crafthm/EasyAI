#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod catalog;
mod commands;
mod db;
mod github;
mod jobs;
mod lifecycle;
mod probe;
mod tools;

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

            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(5));
                let _ = lifecycle::refresh_states(&handle, &db_path, &data_dir, &jobs);
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            github::add_repo,
            commands::list_apps,
            commands::get_app_detail,
            commands::install_app,
            commands::start_app,
            commands::stop_app,
            commands::uninstall_app,
            commands::list_jobs,
            commands::get_job_log,
            commands::preview_recipe,
            commands::import_recipe,
            commands::remove_recipe,
            commands::list_tools
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}