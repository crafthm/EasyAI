use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::catalog::Recipe;
use crate::db::{self, InstalledApp};
use crate::jobs::{self, JobLogger, JobManager};
use crate::probe;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub glyph: String,
    pub color: String,
    pub state: String,
    pub version: Option<String>,
    pub managed: bool,
    pub source_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppDetail {
    pub id: String,
    pub name: String,
    pub description: String,
    pub glyph: String,
    pub color: String,
    pub state: String,
    pub version: Option<String>,
    pub install_dir: Option<String>,
    pub method: Option<String>,
    pub managed: bool,
    pub health_ok: bool,
    pub source_url: Option<String>,
}

pub fn list_apps(
    db_path: &Path,
    app_data: &Path,
    jobs: &JobManager,
) -> Result<Vec<AppSummary>, String> {
    list_apps_with(db_path, app_data, jobs, |recipe| {
        (pid_alive(app_data, &recipe.id), health_ok(recipe))
    })
}

fn list_apps_with(
    db_path: &Path,
    app_data: &Path,
    jobs: &JobManager,
    probe: impl Fn(&Recipe) -> (bool, bool),
) -> Result<Vec<AppSummary>, String> {
    let entries = crate::catalog::entries(app_data)?;
    let installed: HashMap<String, InstalledApp> = db::list_installed(db_path)
        .map_err(|err| format!("Failed to load installed applications: {err}"))?
        .into_iter()
        .map(|app| (app.app_id.clone(), app))
        .collect();

    Ok(entries
        .into_iter()
        .map(|entry| {
            let recipe = entry.recipe;
            let source_url = entry.source.map(|source| source.url);
            let record = installed.get(&recipe.id);
            let (state, version, managed) = match jobs.phase(&recipe.id) {
                Some(phase) => (
                    phase.state().to_string(),
                    record.map(|app| app.version.clone()),
                    record.map(|app| app.managed).unwrap_or(true),
                ),
                None => match record {
                    Some(app) => {
                        let (alive, healthy) = probe(&recipe);
                        let (state, managed) = reconcile(Some(app), alive, healthy);
                        if state != app.state || managed != app.managed {
                            let _ =
                                db::update_installed_state(db_path, &recipe.id, &state, managed);
                        }
                        (state, Some(app.version.clone()), managed)
                    }
                    None => ("not_installed".to_string(), None, true),
                },
            };
            AppSummary {
                id: recipe.id,
                name: recipe.name,
                description: recipe.description,
                glyph: recipe.glyph,
                color: recipe.color,
                state,
                version,
                managed,
                source_url,
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

    let healthy = record.is_some() && health_ok(&recipe);
    let (state, managed) = match (&record, jobs.phase(app_id)) {
        (_, Some(phase)) => (phase.state().to_string(), true),
        (Some(app), None) => reconcile(Some(app), pid_alive(app_data, app_id), healthy),
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
        state,
        version: record.as_ref().map(|app| app.version.clone()),
        install_dir: record.as_ref().map(|app| app.install_dir.clone()),
        method: record.as_ref().map(|app| app.method.clone()),
        managed,
        health_ok: healthy,
        source_url,
    })
}

pub fn reconcile(
    record: Option<&InstalledApp>,
    pid_alive: bool,
    health_ok: bool,
) -> (String, bool) {
    match record {
        None => ("not_installed".to_string(), true),
        Some(_) if pid_alive && health_ok => ("running".to_string(), true),
        Some(_) if pid_alive => ("failed".to_string(), true),
        Some(_) if health_ok => ("running".to_string(), false),
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
    let recipes = crate::catalog::bundled()?;

    for record in installed {
        if jobs.is_active(&record.app_id) {
            continue;
        }
        let Some(recipe) = recipes.iter().find(|recipe| recipe.id == record.app_id) else {
            continue;
        };
        let (state, managed) = reconcile(
            Some(&record),
            pid_alive(app_data, &record.app_id),
            health_ok(recipe),
        );
        if state != record.state || managed != record.managed {
            db::update_installed_state(db_path, &record.app_id, &state, managed)
                .map_err(|err| format!("Failed to update application state: {err}"))?;
            jobs::emit_state(
                app,
                &record.app_id,
                &state,
                Some(&record.version),
                managed,
            );
        }
    }

    Ok(())
}

pub fn install(
    app: &AppHandle,
    db_path: &Path,
    app_data: &Path,
    recipe: &Recipe,
    platform: &str,
    log: &mut JobLogger,
) -> Result<(), String> {
    let install = recipe
        .install
        .as_ref()
        .ok_or_else(|| format!("{} has no install method.", recipe.name))?;
    let service = recipe
        .service
        .as_ref()
        .ok_or_else(|| format!("{} has no service definition.", recipe.name))?;

    log.log(format!("Resolving the latest release of {}.", install.repo));
    let release = fetch_latest_release(&install.repo)?;
    let version = normalize_version(&release.tag_name)?;
    let asset_name = probe::asset_for(recipe, platform)?;

    let downloads = probe::downloads_dir(app_data);
    fs::create_dir_all(&downloads)
        .map_err(|err| format!("Failed to create download directory: {err}"))?;
    let archive = downloads.join(format!("{version}-{asset_name}"));

    let versions = probe::versions_dir(app_data, &recipe.id);
    fs::create_dir_all(&versions)
        .map_err(|err| format!("Failed to create install directory: {err}"))?;
    let staging = versions.join(format!(".staging-{version}"));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|err| format!("Failed to clear staging directory: {err}"))?;
    }

    let install_dir = probe::version_dir(app_data, &recipe.id, &version);

    let result = (|| -> Result<(), String> {
        log.log(format!("Downloading release {version} ({asset_name})."));
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| {
                format!(
                    "Release {version} does not include the expected asset '{asset_name}'."
                )
            })?;
        ensure_archive(&asset.browser_download_url, &archive, asset.size, log)?;

        log.log("Extracting archive.");
        extract(&archive, &staging)?;

        if install_dir.exists() {
            fs::remove_dir_all(&install_dir).map_err(|err| {
                format!("Failed to replace existing install directory: {err}")
            })?;
        }
        fs::rename(&staging, &install_dir)
            .map_err(|err| format!("Failed to activate the new version: {err}"))?;

        let binary = service.command[0].replace("{install_dir}", &install_dir.to_string_lossy());
        log.log(format!("Verifying {binary}."));
        let output = Command::new(&binary)
            .arg("--version")
            .output()
            .map_err(|err| format!("Failed to run the installed binary: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "The installed binary failed its version check (exit {}).",
                output.status
            ));
        }
        let reported = String::from_utf8_lossy(&output.stdout);
        log.log(format!("Version check: {}", reported.trim()));

        db::upsert_installed(
            db_path,
            &InstalledApp {
                app_id: recipe.id.clone(),
                version: version.clone(),
                install_dir: install_dir.to_string_lossy().to_string(),
                method: install.kind.clone(),
                state: "stopped".to_string(),
                managed: true,
            },
        )
        .map_err(|err| format!("Failed to record the installation: {err}"))?;

        jobs::emit_state(app, &recipe.id, "stopped", Some(&version), true);
        log.log(format!("Installed {} {}.", recipe.name, version));
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
        let _ = fs::remove_dir_all(&install_dir);
    } else {
        let _ = fs::remove_file(&archive);
    }
    result
}

pub fn start(
    app: &AppHandle,
    db_path: &Path,
    app_data: &Path,
    recipe: &Recipe,
    log: &mut JobLogger,
) -> Result<(), String> {
    let record = db::get_installed(db_path, &recipe.id)
        .map_err(|err| format!("Failed to load application state: {err}"))?
        .ok_or_else(|| format!("{} is not installed.", recipe.name))?;
    let service = recipe
        .service
        .as_ref()
        .ok_or_else(|| format!("{} has no service definition.", recipe.name))?;

    if pid_alive(app_data, &recipe.id) {
        if health_ok(recipe) {
            db::update_installed_state(db_path, &recipe.id, "running", true)
                .map_err(|err| format!("Failed to update application state: {err}"))?;
            jobs::emit_state(app, &recipe.id, "running", Some(&record.version), true);
            log.log("The application is already running.");
            return Ok(());
        }
        log.log("The existing process is not healthy; restarting it.");
        stop(app, db_path, app_data, recipe, log)?;
    } else if health_ok(recipe) {
        return Err(format!(
            "{} is already running outside EasyAI. Stop it before starting it from EasyAI.",
            recipe.name
        ));
    }

    db::update_installed_state(db_path, &recipe.id, "starting", true)
        .map_err(|err| format!("Failed to update application state: {err}"))?;
    jobs::emit_state(app, &recipe.id, "starting", Some(&record.version), true);

    let install_dir = PathBuf::from(&record.install_dir);
    let command: Vec<String> = service
        .command
        .iter()
        .map(|part| part.replace("{install_dir}", &install_dir.to_string_lossy()))
        .collect();
    if !Path::new(&command[0]).exists() {
        return Err(format!(
            "The installed executable is missing: {}.",
            command[0]
        ));
    }

    let logs = probe::logs_dir(app_data);
    fs::create_dir_all(&logs)
        .map_err(|err| format!("Failed to create log directory: {err}"))?;
    let service_log = logs.join(format!("{}-service.log", recipe.id));
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&service_log)
        .map_err(|err| format!("Failed to open service log: {err}"))?;
    let stderr = stdout
        .try_clone()
        .map_err(|err| format!("Failed to open service log: {err}"))?;

    let mut cmd = Command::new(&command[0]);
    cmd.args(&command[1..])
        .envs(&service.env)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|err| format!("Failed to start {}: {err}", recipe.name))?;
    let pid = child.id() as i32;
    // Reap the child when it exits so liveness checks do not see a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    if let Err(err) = write_pid(app_data, &recipe.id, pid) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        return Err(err);
    }
    log.log(format!(
        "Started process {pid}; waiting for the health check."
    ));

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if health_ok(recipe) {
            db::update_installed_state(db_path, &recipe.id, "running", true)
                .map_err(|err| format!("Failed to update application state: {err}"))?;
            jobs::emit_state(app, &recipe.id, "running", Some(&record.version), true);
            log.log("Health check passed.");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    let _ = fs::remove_file(probe::pid_path(app_data, &recipe.id));
    let _ = db::update_installed_state(db_path, &recipe.id, "failed", true);
    jobs::emit_state(app, &recipe.id, "failed", Some(&record.version), true);
    Err(format!(
        "{} did not become healthy within 30 seconds. See {}.",
        recipe.name,
        service_log.display()
    ))
}

pub fn stop(
    app: &AppHandle,
    db_path: &Path,
    app_data: &Path,
    recipe: &Recipe,
    log: &mut JobLogger,
) -> Result<(), String> {
    let record = db::get_installed(db_path, &recipe.id)
        .map_err(|err| format!("Failed to load application state: {err}"))?
        .ok_or_else(|| format!("{} is not installed.", recipe.name))?;

    match read_pid(app_data, &recipe.id) {
        Some(pid) if process_alive(pid) => {
            log.log(format!("Stopping process {pid}."));
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline && process_alive(pid) {
                std::thread::sleep(Duration::from_millis(100));
            }
            if process_alive(pid) {
                log.log("Process did not exit after SIGTERM; sending SIGKILL.");
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline && process_alive(pid) {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            let _ = fs::remove_file(probe::pid_path(app_data, &recipe.id));
        }
        _ => {
            let _ = fs::remove_file(probe::pid_path(app_data, &recipe.id));
            if health_ok(recipe) {
                return Err(format!(
                    "{} is running outside EasyAI. Stop it manually and try again.",
                    recipe.name
                ));
            }
            log.log("The application was not running.");
        }
    }

    db::update_installed_state(db_path, &recipe.id, "stopped", true)
        .map_err(|err| format!("Failed to update application state: {err}"))?;
    jobs::emit_state(app, &recipe.id, "stopped", Some(&record.version), true);
    log.log("Stopped.");
    Ok(())
}

pub fn uninstall(
    app: &AppHandle,
    db_path: &Path,
    app_data: &Path,
    recipe: &Recipe,
    log: &mut JobLogger,
) -> Result<(), String> {
    let record = db::get_installed(db_path, &recipe.id)
        .map_err(|err| format!("Failed to load application state: {err}"))?
        .ok_or_else(|| format!("{} is not installed.", recipe.name))?;

    if pid_alive(app_data, &recipe.id) {
        log.log("Stopping the application before uninstalling.");
        stop(app, db_path, app_data, recipe, log)?;
    } else if health_ok(recipe) {
        return Err(format!(
            "{} is running outside EasyAI. Stop it before uninstalling.",
            recipe.name
        ));
    }

    log.log("Removing installed files.");
    let app_dir = probe::app_dir(app_data, &recipe.id);
    if app_dir.exists() {
        fs::remove_dir_all(&app_dir)
            .map_err(|err| format!("Failed to remove {}: {err}", app_dir.display()))?;
    }
    let _ = fs::remove_file(probe::pid_path(app_data, &recipe.id));

    db::delete_installed(db_path, &recipe.id)
        .map_err(|err| format!("Failed to remove the installed record: {err}"))?;
    jobs::emit_state(app, &recipe.id, "not_installed", None, true);
    log.log(format!(
        "Uninstalled {} {}; user data was kept.",
        recipe.name, record.version
    ));
    if let Some(uninstall) = &recipe.uninstall {
        if !uninstall.keep_paths.is_empty() {
            log.log(format!(
                "Preserved user data: {}.",
                uninstall.keep_paths.join(", ")
            ));
        }
    }
    Ok(())
}

pub fn normalize_version(tag: &str) -> Result<String, String> {
    let version = tag.trim().trim_start_matches(['v', 'V']);
    if version.is_empty()
        || !version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        return Err(format!("The release tag '{tag}' is not a usable version."));
    }
    Ok(version.to_string())
}

fn health_ok(recipe: &Recipe) -> bool {
    let Some(service) = &recipe.service else {
        return false;
    };
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
    else {
        return false;
    };
    client
        .get(&service.health_url)
        .send()
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

fn read_pid(app_data: &Path, app_id: &str) -> Option<i32> {
    fs::read_to_string(probe::pid_path(app_data, app_id))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn write_pid(app_data: &Path, app_id: &str, pid: i32) -> Result<(), String> {
    let path = probe::pid_path(app_data, app_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create run directory: {err}"))?;
    }
    fs::write(&path, format!("{pid}\n"))
        .map_err(|err| format!("Failed to record process id: {err}"))
}

fn pid_alive(app_data: &Path, app_id: &str) -> bool {
    read_pid(app_data, app_id)
        .map(process_alive)
        .unwrap_or(false)
}

fn process_alive(pid: i32) -> bool {
    pid > 0 && unsafe { libc::kill(pid, 0) == 0 }
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

fn fetch_latest_release(repo: &str) -> Result<Release, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("EasyAI/0.1")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| format!("Failed to create HTTP client: {err}"))?;
    let response = client
        .get(format!("https://api.github.com/repos/{repo}/releases/latest"))
        .send()
        .map_err(|err| format!("Could not reach GitHub: {err}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            404 => format!("No releases were found for {repo}."),
            403 => "GitHub API rate limit exceeded. Try again later.".to_string(),
            _ => format!("GitHub API returned {status}."),
        });
    }
    response
        .json::<Release>()
        .map_err(|err| format!("Invalid release metadata from GitHub: {err}"))
}

fn ensure_archive(
    url: &str,
    archive: &Path,
    expected_size: u64,
    log: &mut JobLogger,
) -> Result<(), String> {
    if expected_size > 0 {
        if let Ok(metadata) = fs::metadata(archive) {
            if metadata.len() == expected_size {
                log.log("Using the previously downloaded archive.");
                return Ok(());
            }
        }
    }
    log.log("Downloading the release archive.");
    download(url, archive, log)
}

fn download(url: &str, destination: &Path, log: &mut JobLogger) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("EasyAI/0.1")
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|err| format!("Failed to create HTTP client: {err}"))?;
    let mut response = client
        .get(url)
        .send()
        .map_err(|err| format!("Download failed: could not reach the release server: {err}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Download failed: the release server returned {status}."));
    }

    let total = response.content_length();
    let mut file = File::create(destination)
        .map_err(|err| format!("Download failed: cannot create the archive file: {err}"))?;
    let mut buffer = [0u8; 64 * 1024];
    let mut written = 0u64;
    let mut reported = 0u64;

    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|err| format!("Download failed while reading the archive: {err}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|err| format!("Download failed while writing the archive: {err}"))?;
        written += read as u64;
        if written - reported >= 50 * 1024 * 1024 || Some(written) == total {
            log.log(match total {
                Some(total) => format!(
                    "Downloaded {} / {} MB.",
                    written / 1_048_576,
                    total / 1_048_576
                ),
                None => format!("Downloaded {} MB.", written / 1_048_576),
            });
            reported = written;
        }
    }

    file.flush()
        .map_err(|err| format!("Download failed while flushing the archive: {err}"))?;
    Ok(())
}

fn extract(archive: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|err| format!("Failed to create extraction directory: {err}"))?;
    let file = File::open(archive)
        .map_err(|err| format!("Failed to open the downloaded archive: {err}"))?;

    if archive.extension().and_then(|ext| ext.to_str()) == Some("zst") {
        let decoder = zstd::stream::read::Decoder::new(file)
            .map_err(|err| format!("Failed to read the zstd archive: {err}"))?;
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(destination)
            .map_err(|err| format!("Failed to extract the archive: {err}"))
    } else {
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(destination)
            .map_err(|err| format!("Failed to extract the archive: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{list_apps_with, normalize_version, reconcile};
    use crate::db::{self, InstalledApp};
    use crate::jobs::JobManager;

    fn record() -> InstalledApp {
        InstalledApp {
            app_id: "ollama".to_string(),
            version: "1.0.0".to_string(),
            install_dir: "/tmp/ollama".to_string(),
            method: "github-release-binary".to_string(),
            state: "stopped".to_string(),
            managed: true,
        }
    }

    #[test]
    fn normalizes_release_tags() {
        assert_eq!(normalize_version("v1.2.3").unwrap(), "1.2.3");
        assert_eq!(normalize_version("1.2.3").unwrap(), "1.2.3");
        assert!(normalize_version("v").is_err());
        assert!(normalize_version("v1.2.3;rm").is_err());
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

        let apps = list_apps_with(&path, &data_dir, &jobs, |_| (false, false)).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "ollama");
        assert_eq!(apps[0].state, "not_installed");
        assert!(apps[0].version.is_none());
        assert!(apps[0].source_url.is_none());

        db::upsert_installed(&path, &record()).unwrap();
        let apps = list_apps_with(&path, &data_dir, &jobs, |_| (false, false)).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].state, "stopped");
        assert_eq!(apps[0].version.as_deref(), Some("1.0.0"));

        let mut ghost = record();
        ghost.app_id = "ghost".to_string();
        db::upsert_installed(&path, &ghost).unwrap();
        let apps = list_apps_with(&path, &data_dir, &jobs, |_| (false, false)).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "ollama");

        std::fs::remove_dir_all(&data_dir).unwrap();
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn reconciliation_covers_every_combination() {
        let app = record();
        assert_eq!(reconcile(None, false, false), ("not_installed".to_string(), true));
        assert_eq!(reconcile(Some(&app), true, true), ("running".to_string(), true));
        assert_eq!(reconcile(Some(&app), true, false), ("failed".to_string(), true));
        assert_eq!(
            reconcile(Some(&app), false, true),
            ("running".to_string(), false)
        );
        assert_eq!(
            reconcile(Some(&app), false, false),
            ("stopped".to_string(), true)
        );
    }
}