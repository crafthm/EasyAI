use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::catalog::{self, Method, Recipe, RecipePreview};
use crate::db::{self, JobRecord};
use crate::jobs::{self, JobLogger, Phase};
use crate::lifecycle::{self, AppDetail, AppSummary};
use crate::probe;
use crate::resolve::{self, InstallPlan, MethodSettings, Preferences, TIERS};
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

fn load_method_settings(db_path: &Path) -> MethodSettings {
    db::get_setting(db_path, "method_tiers")
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn plan_for(
    db_path: &Path,
    app_data: &Path,
    app_id: &str,
    method_id: Option<String>,
    allow_advanced: bool,
    allow_elevation: bool,
) -> Result<(Recipe, Method, InstallPlan), String> {
    let recipe = catalog::find(app_data, app_id)?.recipe;
    let profile = probe::collect(app_data)?;
    let existing = probe::find_existing(&recipe, &profile, app_data);
    let preferences = Preferences {
        allow_elevation,
        allow_advanced,
        preferred_method: method_id,
        replace_existing: false,
        settings: load_method_settings(db_path),
    };
    let plan = resolve::resolve(&recipe, &profile, &preferences, &existing)?;
    let method = recipe
        .method(&plan.method_id)
        .cloned()
        .ok_or_else(|| format!("Unknown method '{}'.", plan.method_id))?;
    Ok((recipe, method, plan))
}

fn installed_method(
    db_path: &Path,
    app_data: &Path,
    app_id: &str,
) -> Result<(Recipe, Method, String), String> {
    let recipe = catalog::find(app_data, app_id)?.recipe;
    let record = db::get_installed(db_path, app_id)
        .map_err(|err| format!("Failed to load application state: {err}"))?
        .ok_or_else(|| "The application is not installed.".to_string())?;
    let method = lifecycle::method_for(&recipe, &record)
        .cloned()
        .ok_or_else(|| "The installed method is no longer present in the recipe.".to_string())?;
    let method_id = method.id.clone();
    Ok((recipe, method, method_id))
}

async fn run_job<F>(
    app: AppHandle,
    state: State<'_, AppState>,
    tool: &'static str,
    job_kind: &'static str,
    phase: Phase,
    app_id: String,
    method_id: String,
    plan_json: Option<String>,
    origin: Option<String>,
    confirmed: Option<bool>,
    work: F,
) -> Result<(), String>
where
    F: FnOnce(&AppHandle, &Path, &Path, &Recipe, &Method, &mut JobLogger) -> Result<(), String>
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
    let job_method_id = method_id.clone();
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
            Some(&job_method_id),
            plan_json.as_deref(),
        )?;
        let result = (|| {
            let recipe = catalog::find(&app_data, &job_app_id)?.recipe;
            let method = recipe
                .method(&job_method_id)
                .ok_or_else(|| format!("Unknown method '{job_method_id}'."))?;
            work(&app_for_job, &db_path, &app_data, &recipe, method, &mut log)
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
pub async fn plan_install(
    state: State<'_, AppState>,
    app_id: String,
    method_id: Option<String>,
    allow_advanced: Option<bool>,
    allow_elevation: Option<bool>,
) -> Result<InstallPlan, String> {
    let app_data = state.app_data_dir.clone();
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        plan_for(
            &db_path,
            &app_data,
            &app_id,
            method_id,
            allow_advanced.unwrap_or(false),
            allow_elevation.unwrap_or(false),
        )
        .map(|(_, _, plan)| plan)
    })
    .await
    .map_err(|err| format!("Failed to plan the installation: {err}"))?
}

#[tauri::command]
pub async fn install_app(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: String,
    method_id: Option<String>,
    allow_advanced: Option<bool>,
    allow_elevation: Option<bool>,
    origin: Option<String>,
    confirmed: Option<bool>,
) -> Result<(), String> {
    let app_data = state.app_data_dir.clone();
    let db_path = state.db_path.clone();
    let requested = app_id.clone();
    let (method_id, plan_json) = tauri::async_runtime::spawn_blocking(move || {
        let (_, method, plan) = plan_for(
            &db_path,
            &app_data,
            &requested,
            method_id,
            allow_advanced.unwrap_or(false),
            allow_elevation.unwrap_or(false),
        )?;
        if plan.action == "adopt" {
            return Err(format!(
                "The application is already installed through method '{}'. Use the existing installation.",
                plan.method_id
            ));
        }
        if plan.needs_elevation && !allow_elevation.unwrap_or(false) {
            return Err("This method requires elevation, which is not enabled.".to_string());
        }
        let plan_json = serde_json::to_string(&plan)
            .map_err(|err| format!("Failed to serialize the install plan: {err}"))?;
        Ok::<_, String>((method.id, plan_json))
    })
    .await
    .map_err(|err| format!("Failed to plan the installation: {err}"))??;

    run_job(
        app,
        state,
        "install_app",
        "install",
        Phase::Installing,
        app_id,
        method_id,
        Some(plan_json),
        origin,
        confirmed,
        |app, db_path, app_data, recipe, method, log| {
            lifecycle::install(app, db_path, app_data, recipe, method, log)
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
    let (_, _, method_id) =
        installed_method(&state.db_path, &state.app_data_dir, &app_id)?;
    run_job(
        app,
        state,
        "start_app",
        "start",
        Phase::Starting,
        app_id,
        method_id,
        None,
        origin,
        confirmed,
        |app, db_path, app_data, recipe, method, log| {
            lifecycle::start(app, db_path, app_data, recipe, method, log)
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
    let (_, _, method_id) =
        installed_method(&state.db_path, &state.app_data_dir, &app_id)?;
    run_job(
        app,
        state,
        "stop_app",
        "stop",
        Phase::Stopping,
        app_id,
        method_id,
        None,
        origin,
        confirmed,
        |app, db_path, app_data, recipe, method, log| {
            lifecycle::stop(app, db_path, app_data, recipe, method, log)
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
    let (_, _, method_id) =
        installed_method(&state.db_path, &state.app_data_dir, &app_id)?;
    run_job(
        app,
        state,
        "uninstall_app",
        "uninstall",
        Phase::Uninstalling,
        app_id,
        method_id,
        None,
        origin,
        confirmed,
        |app, db_path, app_data, recipe, method, log| {
            lifecycle::uninstall(app, db_path, app_data, recipe, method, log)
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
pub async fn open_url(
    state: State<'_, AppState>,
    app_id: String,
    url: String,
) -> Result<(), String> {
    let app_data = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let recipe = catalog::find(&app_data, &app_id)?.recipe;
        let usage = recipe
            .usage
            .as_ref()
            .ok_or_else(|| "This application declares no usage URLs.".to_string())?;
        if !usage.declares(&url) {
            return Err("This URL is not declared by the application's recipe.".to_string());
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("Only http and https URLs can be opened.".to_string());
        }
        open_external(&url)
    })
    .await
    .map_err(|err| format!("Failed to open the URL: {err}"))?
}

fn open_external(url: &str) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    } else {
        Command::new("xdg-open")
    };
    command
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("Could not open the URL: {err}"))
}

#[tauri::command]
pub async fn get_method_settings(
    state: State<'_, AppState>,
) -> Result<MethodSettings, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || Ok(load_method_settings(&db_path)))
        .await
        .map_err(|err| format!("Failed to load method settings: {err}"))?
}

#[tauri::command]
pub async fn save_method_settings(
    state: State<'_, AppState>,
    settings: MethodSettings,
    origin: Option<String>,
    confirmed: Option<bool>,
) -> Result<(), String> {
    authorize("save_method_settings", origin, confirmed)?;
    let mut seen = std::collections::HashSet::new();
    if settings.order.len() != TIERS.len()
        || settings
            .order
            .iter()
            .any(|tier| !TIERS.contains(&tier.as_str()))
        || !settings.order.iter().all(|tier| seen.insert(tier.clone()))
    {
        return Err("The tier order must contain T0, T1, T2, T3, and T4 exactly once.".to_string());
    }
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let value = serde_json::to_string(&settings)
            .map_err(|err| format!("Failed to serialize method settings: {err}"))?;
        db::set_setting(&db_path, "method_tiers", &value)
            .map_err(|err| format!("Failed to save method settings: {err}"))
    })
    .await
    .map_err(|err| format!("Failed to save method settings: {err}"))?
}

#[tauri::command]
pub async fn get_bom(
    state: State<'_, AppState>,
    app_id: String,
) -> Result<crate::methods::Bom, String> {
    let app_data = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::methods::read_bom(&app_data, &app_id)
    })
    .await
    .map_err(|err| format!("Failed to read the bill of materials: {err}"))?
}

#[tauri::command]
pub fn list_tools() -> Vec<ToolSpec> {
    tools::TOOLS.to_vec()
}