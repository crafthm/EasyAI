#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod db;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            db::init(&data_dir.join("startai.db"))?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}