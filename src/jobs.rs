use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::db;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Installing,
    Starting,
    Stopping,
    Uninstalling,
}

impl Phase {
    pub fn state(self) -> &'static str {
        match self {
            Phase::Installing => "installing",
            Phase::Starting => "starting",
            Phase::Stopping => "stopping",
            Phase::Uninstalling => "uninstalling",
        }
    }
}

pub struct JobManager {
    active: Mutex<HashMap<String, Phase>>,
}

impl JobManager {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(HashMap::new()),
        }
    }

    pub fn try_begin(self: &Arc<Self>, app_id: &str, phase: Phase) -> Result<JobGuard, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "Job state is unavailable.".to_string())?;
        if let Some(existing) = active.get(app_id) {
            return Err(format!(
                "'{app_id}' is already busy ({}). Wait for the current operation to finish.",
                existing.state()
            ));
        }
        active.insert(app_id.to_string(), phase);
        Ok(JobGuard {
            manager: Arc::clone(self),
            app_id: app_id.to_string(),
        })
    }

    pub fn phase(&self, app_id: &str) -> Option<Phase> {
        self.active
            .lock()
            .ok()
            .and_then(|active| active.get(app_id).copied())
    }

    pub fn is_active(&self, app_id: &str) -> bool {
        self.phase(app_id).is_some()
    }
}

pub struct JobGuard {
    manager: Arc<JobManager>,
    app_id: String,
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.manager.active.lock() {
            active.remove(&self.app_id);
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateEvent {
    pub app_id: String,
    pub state: String,
    pub version: Option<String>,
    pub managed: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLogEvent {
    pub job_id: i64,
    pub app_id: String,
    pub line: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobFinishedEvent {
    pub job_id: i64,
    pub app_id: String,
    pub state: String,
    pub error: Option<String>,
}

pub fn emit_state(
    app: &AppHandle,
    app_id: &str,
    state: &str,
    version: Option<&str>,
    managed: bool,
) {
    let _ = app.emit(
        "app-state-changed",
        AppStateEvent {
            app_id: app_id.to_string(),
            state: state.to_string(),
            version: version.map(str::to_string),
            managed,
        },
    );
}

struct LogFile {
    file: File,
}

impl LogFile {
    fn create(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create log directory: {err}"))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|err| format!("Failed to open log file {}: {err}", path.display()))?;
        Ok(Self { file })
    }

    fn write_line(&mut self, line: &str) -> Result<(), String> {
        writeln!(self.file, "{line}").map_err(|err| format!("Failed to write log: {err}"))?;
        self.file
            .flush()
            .map_err(|err| format!("Failed to flush log: {err}"))
    }
}

pub struct JobLogger {
    pub job_id: i64,
    pub app_id: String,
    db_path: PathBuf,
    app: AppHandle,
    file: LogFile,
}

impl JobLogger {
    pub fn create(
        app: &AppHandle,
        db_path: &Path,
        logs_dir: &Path,
        app_id: &str,
        kind: &str,
        origin: &str,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(logs_dir)
            .map_err(|err| format!("Failed to create log directory: {err}"))?;

        let job_id = db::create_job(db_path, app_id, kind, origin, "")
            .map_err(|err| format!("Failed to record job: {err}"))?;
        let log_path = logs_dir.join(format!("{job_id}.log"));
        db::set_job_log_path(db_path, job_id, &log_path.to_string_lossy())
            .map_err(|err| format!("Failed to record job log path: {err}"))?;

        let mut logger = Self {
            job_id,
            app_id: app_id.to_string(),
            db_path: db_path.to_path_buf(),
            app: app.clone(),
            file: LogFile::create(&log_path)?,
        };
        logger.log(format!("[{kind}] started (origin: {origin})"));
        Ok(logger)
    }

    pub fn log(&mut self, line: impl AsRef<str>) {
        let line = line.as_ref();
        if let Err(err) = self.file.write_line(line) {
            let _ = self.file.write_line(&format!("(failed to write log: {err})"));
        }
        let _ = self.app.emit(
            "job-log",
            JobLogEvent {
                job_id: self.job_id,
                app_id: self.app_id.clone(),
                line: line.to_string(),
            },
        );
    }

    pub fn finish(mut self, result: Result<(), String>) -> Result<(), String> {
        let (state, error) = match &result {
            Ok(()) => ("succeeded", None),
            Err(err) => ("failed", Some(err.clone())),
        };
        match &error {
            Some(err) => self.log(format!("FAILED: {err}")),
            None => self.log("Finished."),
        }

        db::finish_job(&self.db_path, self.job_id, state, error.as_deref())
            .map_err(|err| format!("Failed to record job result: {err}"))?;
        let _ = self.app.emit(
            "job-finished",
            JobFinishedEvent {
                job_id: self.job_id,
                app_id: self.app_id.clone(),
                state: state.to_string(),
                error,
            },
        );

        result
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::Arc;

    use super::{JobManager, LogFile, Phase};

    #[test]
    fn rejects_second_job_for_same_app() {
        let manager = Arc::new(JobManager::new());
        let _first = manager.try_begin("ollama", Phase::Installing).unwrap();
        let err = match manager.try_begin("ollama", Phase::Starting) {
            Ok(_) => panic!("expected the second job to be rejected"),
            Err(err) => err,
        };
        assert!(err.contains("already busy"), "{err}");
    }

    #[test]
    fn accepts_jobs_for_different_apps() {
        let manager = Arc::new(JobManager::new());
        let _first = manager.try_begin("ollama", Phase::Installing).unwrap();
        let _second = manager.try_begin("other", Phase::Installing).unwrap();
        assert!(manager.is_active("ollama"));
        assert!(manager.is_active("other"));
    }

    #[test]
    fn guard_releases_app_when_dropped() {
        let manager = Arc::new(JobManager::new());
        {
            let _guard = manager.try_begin("ollama", Phase::Stopping).unwrap();
            assert!(manager.is_active("ollama"));
        }
        assert!(!manager.is_active("ollama"));
    }

    #[test]
    fn log_file_persists_lines() {
        let path = std::env::temp_dir().join(format!(
            "easyai-job-log-test-{}.log",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let mut log = LogFile::create(&path).unwrap();
            log.write_line("first").unwrap();
            log.write_line("second").unwrap();
        }
        let mut contents = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert_eq!(contents, "first\nsecond\n");
        std::fs::remove_file(&path).unwrap();
    }
}