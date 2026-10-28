use std::path::Path;
use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::catalog::{self, Recipe, RecipePreview};
use crate::db::{self, JobRecord};
use crate::jobs::{self, JobLogger, Phase};
use crate::lifecycle::{self, AppDetail, AppSummary};
use crate::probe;
use crate::tools::{self, ToolSpec};
use crate::AppState;

fn authorize(
    tool: &str,
    origin: Option<String>,
    confirmed: Option<bool>,
) -> Result<String, String> {
    let origin = origin.unwrap_or_else(|| "ui".to_string());
    let confirmed = confirmed.unwrap_or(origin == "ui");
    tools::authorize(tool, &origin, confirmed)?;
    Ok(origin)
}

async fn run_job<F>(
    app: AppHandle,
    state: State<'_, AppState>,
    tool: &'static str,
    job_kind: &'static str,
    phase: Phase,
    app_id: String,
    origin: Option<String>,
    confirmed: Option<bool>,
    work: F,
) -> Result<(), String>
where
    F: FnOnce(&AppHandle, &Path, &Path, &Recipe, &mut JobLogger) -> Result<(), String>
        + Send
        + 'static,
{
    let origin = authorize(tool, origin, confirmed)?;
    let db_path = state.db_path.clone();
    let app_data = state.app_data_dir.clone();
    let jobs = Arc::clone(&state.jobs);

    let guard = jobs.try_begin(&app_id, phase)?;
    jobs::emit_state(&app, &app_id, phase.state(), None, true);

    let job_app_id = app_id.clone();
    let app_for_job = app.clone();
    let jobs_for_job = Arc::clone(&jobs);
    tauri::async_runtime::spawn_blocking(move || {
        let guard = guard;
        let mut log = JobLogger::create(
            &app_for_job,
            &db_path,
            &probe::logs_dir(&app_data),
            &job_app_id,
            job_kind,
            &origin,
        )?;
        let result = (|| {
            let recipe = catalog::find(&app_data, &job_app_id)?.recipe;
            work(&app_for_job, &db_path, &app_data, &recipe, &mut log)
        })();
        if result.is_err() {
            drop(guard);
            match phase {
                Phase::Installing => jobs::emit_state(
                    &app_for_job,
                    &job_app_id,
                    "not_installed",
                    None,
                    true,
                ),
                _ => {
                    let _ = lifecycle::refresh_states(
                        &app_for_job,
                        &db_path,
                        &app_data,
                        &jobs_for_job,
                    );
                }
            }
        }
        log.finish(result)
    })
    .await
    .map_err(|err| format!("{job_kind} task failed: {err}"))?
}

#[tauri::command]
pub async fn list_apps(state: State<'_, AppState>) -> Result<Vec<AppSummary>, String> {
    let db_path = state.db_path.clone();
    let app_data = state.app_data_dir.clone();
    let jobs = Arc::clone(&state.jobs);
    tauri::async_runtime::spawn_blocking(move || {
        lifecycle::list_apps(&db_path, &app_data, &jobs)
    })
    .await
    .map_err(|err| format!("Failed to list applications: {err}"))?
}

#[tauri::command]
pub async fn get_app_detail(
    state: State<'_, AppState>,
    app_id: String,
) -> Result<AppDetail, String> {
    let db_path = state.db_path.clone();
    let app_data = state.app_data_dir.clone();
    let jobs = Arc::clone(&state.jobs);
    tauri::async_runtime::spawn_blocking(move || {
        lifecycle::get_app_detail(&db_path, &app_data, &jobs, &app_id)
    })
    .await
    .map_err(|err| format!("Failed to load application details: {err}"))?
}

#[tauri::command]
pub async fn install_app(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: String,
    origin: Option<String>,
    confirmed: Option<bool>,
) -> Result<(), String> {
    run_job(
        app,
        state,
        "install_app",
        "install",
        Phase::Installing,
        app_id,
        origin,
        confirmed,
        |app, db_path, app_data, recipe, log| {
            let platform = probe::current_platform_id()?;
            lifecycle::install(app, db_path, app_data, recipe, &platform, log)
        },
    )
    .await
}

#[tauri::command]
pub async fn start_app(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: String,
    origin: Option<String>,
    confirmed: Option<bool>,
) -> Result<(), String> {
    run_job(
        app,
        state,
        "start_app",
        "start",
        Phase::Starting,
        app_id,
        origin,
        confirmed,
        |app, db_path, app_data, recipe, log| {
            lifecycle::start(app, db_path, app_data, recipe, log)
        },
    )
    .await
}

#[tauri::command]
pub async fn stop_app(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: String,
    origin: Option<String>,
    confirmed: Option<bool>,
) -> Result<(), String> {
    run_job(
        app,
        state,
        "stop_app",
        "stop",
        Phase::Stopping,
        app_id,
        origin,
        confirmed,
        |app, db_path, app_data, recipe, log| {
            lifecycle::stop(app, db_path, app_data, recipe, log)
        },
    )
    .await
}

#[tauri::command]
pub async fn uninstall_app(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: String,
    origin: Option<String>,
    confirmed: Option<bool>,
) -> Result<(), String> {
    run_job(
        app,
        state,
        "uninstall_app",
        "uninstall",
        Phase::Uninstalling,
        app_id,
        origin,
        confirmed,
        |app, db_path, app_data, recipe, log| {
            lifecycle::uninstall(app, db_path, app_data, recipe, log)
        },
    )
    .await
}

#[tauri::command]
pub async fn list_jobs(
    state: State<'_, AppState>,
    app_id: String,
) -> Result<Vec<JobRecord>, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        db::list_jobs(&db_path, &app_id).map_err(|err| format!("Failed to load jobs: {err}"))
    })
    .await
    .map_err(|err| format!("Failed to load jobs: {err}"))?
}

#[tauri::command]
pub async fn get_job_log(
    state: State<'_, AppState>,
    job_id: i64,
) -> Result<String, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let job = db::get_job(&db_path, job_id)
            .map_err(|err| format!("Failed to load the job: {err}"))?
            .ok_or_else(|| "Job not found.".to_string())?;
        std::fs::read_to_string(&job.log_path)
            .map_err(|err| format!("Failed to read the job log: {err}"))
    })
    .await
    .map_err(|err| format!("Failed to read the job log: {err}"))?
}

#[tauri::command]
pub async fn preview_recipe(
    state: State<'_, AppState>,
    url: String,
) -> Result<RecipePreview, String> {
    let app_data = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || catalog::preview_recipe(&app_data, &url))
        .await
        .map_err(|err| format!("Failed to preview the recipe: {err}"))?
}

#[tauri::command]
pub async fn import_recipe(
    state: State<'_, AppState>,
    url: String,
    sha256: String,
    origin: Option<String>,
    confirmed: Option<bool>,
) -> Result<(), String> {
    authorize("import_recipe", origin, confirmed)?;
    let app_data = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        catalog::import_recipe(&app_data, &url, &sha256).map(|_| ())
    })
    .await
    .map_err(|err| format!("Failed to import the recipe: {err}"))?
}

#[tauri::command]
pub async fn remove_recipe(
    state: State<'_, AppState>,
    app_id: String,
    origin: Option<String>,
    confirmed: Option<bool>,
) -> Result<(), String> {
    authorize("remove_recipe", origin, confirmed)?;
    let app_data = state.app_data_dir.clone();
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if db::get_installed(&db_path, &app_id)
            .map_err(|err| format!("Failed to load application state: {err}"))?
            .is_some()
        {
            return Err("Uninstall the application before removing its recipe.".to_string());
        }
        catalog::remove_recipe(&app_data, &app_id)
    })
    .await
    .map_err(|err| format!("Failed to remove the recipe: {err}"))?
}

#[tauri::command]
pub fn list_tools() -> Vec<ToolSpec> {
    tools::TOOLS.to_vec()
}