#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod db;
mod github;

use std::path::PathBuf;

use tauri::Manager;

pub struct AppState {
    pub db_path: PathBuf,
    pub repos_dir: PathBuf,
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("startai.db");
            db::init(&db_path)?;
            let repos_dir = data_dir.join("repos");
            std::fs::create_dir_all(&repos_dir)?;
            app.manage(AppState { db_path, repos_dir });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![github::add_repo])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}