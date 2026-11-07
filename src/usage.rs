use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::catalog::{Method, Recipe, Usage, UsageItem};
use crate::db::{self, InstalledApp, UsageCheckRecord};
use crate::methods;
use crate::probe;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageItemState {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub value: String,
    pub state: String,
    pub status: Option<String>,
    pub message: Option<String>,
    pub checked_at: Option<String>,
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    pub app_id: String,
    pub method_id: String,
    pub version: String,
    pub checked_at: Option<String>,
    pub items: Vec<UsageItemState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedEntry {
    pub kind: String,
    pub url: String,
    pub title: Option<String>,
    pub confidence: String,
    pub evidence: String,
}

pub fn effective_usage(recipe: &Recipe, method: &Method) -> Option<Usage> {
    recipe
        .effective_usage(method)
        .or_else(|| recipe.usage.clone())
}

pub fn run_check(
    db_path: &Path,
    recipe: &Recipe,
    method: &Method,
    record: &InstalledApp,
    running: bool,
) -> UsageReport {
    let usage = effective_usage(recipe, method).unwrap_or_default();
    let items = usage.items();
    let checked_at = now();
    let mut states = Vec::new();

    for item in items {
        let (status, message, details) = if let Some(error) = static_error(&item) {
            ("failed", Some(error), Some(json!({ "error": "static check failed" })))
        } else if item.kind == "launch" {
            probe_launch(&item.value, &record.install_dir)
        } else if !running {
            (
                "skipped",
                Some("Application is not running.".to_string()),
                None,
            )
        } else if item.is_url {
            probe_url(&item.value)
        } else if item.kind == "cli" {
            probe_cli(&item.value, &record.install_dir)
        } else {
            ("skipped", Some("Unsupported usage item.".to_string()), None)
        };

        let details_json = serde_json::to_string(&details).unwrap_or_else(|_| "{}".to_string());
        let _ = db::upsert_usage_check(
            db_path,
            &record.app_id,
            &method.id,
            &record.version,
            &item.id,
            status,
            &checked_at,
            &details_json,
        );

        states.push(UsageItemState {
            id: item.id,
            kind: item.kind,
            label: item.label,
            value: item.value,
            state: state_for_status(status).to_string(),
            status: Some(status.to_string()),
            message,
            checked_at: Some(checked_at.clone()),
            details,
        });
    }

    UsageReport {
        app_id: record.app_id.clone(),
        method_id: method.id.clone(),
        version: record.version.clone(),
        checked_at: Some(checked_at),
        items: states,
    }
}

pub fn report(
    db_path: &Path,
    recipe: &Recipe,
    method: &Method,
    record: &InstalledApp,
) -> UsageReport {
    let usage = effective_usage(recipe, method).unwrap_or_default();
    let records = db::list_usage_checks(db_path, &record.app_id, &method.id).unwrap_or_default();
    let mut items = Vec::new();
    let mut latest: Option<String> = None;

    for item in usage.items() {
        let exact = records
            .iter()
            .find(|check| check.item_id == item.id && check.version == record.version);
        let other = records
            .iter()
            .find(|check| check.item_id == item.id && check.version != record.version);
        let check = exact.or(other);
        let (state, message) = classify(exact, other, &record.version);
        if let Some(check) = check {
            if latest
                .as_ref()
                .map(|value| check.checked_at > *value)
                .unwrap_or(true)
            {
                latest = Some(check.checked_at.clone());
            }
        }
        items.push(UsageItemState {
            id: item.id,
            kind: item.kind,
            label: item.label,
            value: item.value,
            state: state.to_string(),
            status: check.map(|check| check.status.clone()),
            message,
            checked_at: check.map(|check| check.checked_at.clone()),
            details: check
                .and_then(|check| serde_json::from_str::<Value>(&check.details_json).ok()),
        });
    }

    UsageReport {
        app_id: record.app_id.clone(),
        method_id: method.id.clone(),
        version: record.version.clone(),
        checked_at: latest,
        items,
    }
}

fn classify(
    exact: Option<&UsageCheckRecord>,
    other: Option<&UsageCheckRecord>,
    version: &str,
) -> (&'static str, Option<String>) {
    if let Some(record) = exact {
        return if record.status == "ok" {
            ("verified", None)
        } else if record.status == "skipped" {
            ("unverified", record_message(record))
        } else {
            ("failed", record_message(record))
        };
    }
    if let Some(record) = other {
        return (
            "stale",
            Some(format!(
                "Last checked against version {}.",
                if record.version.is_empty() {
                    "unknown"
                } else {
                    record.version.as_str()
                }
            )),
        );
    }
    ("unverified", Some(format!("No check for version {version}.")))
}

fn record_message(record: &UsageCheckRecord) -> Option<String> {
    serde_json::from_str::<Value>(&record.details_json)
        .ok()
        .and_then(|value| value.get("error").and_then(Value::as_str).map(str::to_string))
}

fn state_for_status(status: &str) -> &'static str {
    match status {
        "ok" => "verified",
        "failed" => "failed",
        _ => "unverified",
    }
}

fn static_error(item: &UsageItem) -> Option<String> {
    if item.label.trim().is_empty() {
        return Some("Usage item has an empty label.".to_string());
    }
    if item.is_url {
        let Ok(url) = reqwest::Url::parse(&item.value) else {
            return Some("URL is not valid.".to_string());
        };
        if !matches!(url.scheme(), "http" | "https") {
            return Some("URL must use http or https.".to_string());
        }
        return None;
    }
    if item.kind == "cli" || item.kind == "launch" {
        let first = item.value.split_whitespace().next().unwrap_or("");
        if first.is_empty() {
            return Some("CLI command is empty.".to_string());
        }
        if first.contains([';', '|', '&', '$', '`', '<', '>']) {
            return Some("CLI command contains shell metacharacters.".to_string());
        }
    }
    None
}

fn probe_url(url: &str) -> (&'static str, Option<String>, Option<Value>) {
    let started = Instant::now();
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(err) => return ("failed", Some(err.to_string()), None),
    };
    let response = match client.get(url).send() {
        Ok(response) => response,
        Err(err) => {
            return (
                "failed",
                Some(format!("Request failed: {err}")),
                Some(json!({ "error": err.to_string() })),
            )
        }
    };
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut body = Vec::new();
    let _ = response
        .take(64 * 1024 + 1)
        .read_to_end(&mut body);
    let text = String::from_utf8_lossy(&body);
    let title = if content_type.contains("text/html") {
        extract_title(&text)
    } else {
        None
    };
    let latency = started.elapsed().as_millis() as u64;
    let details = json!({
        "httpStatus": status.as_u16(),
        "latencyMs": latency,
        "contentType": content_type,
        "title": title,
    });
    if status.is_success() {
        ("ok", None, Some(details))
    } else {
        (
            "failed",
            Some(format!("HTTP {status}")),
            Some(json!({
                "httpStatus": status.as_u16(),
                "latencyMs": latency,
                "error": format!("HTTP {status}"),
            })),
        )
    }
}

fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title>")? + "<title>".len();
    let end = lower[start..].find("</title>")? + start;
    let title = html[start..end].trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

fn probe_launch(
    command: &str,
    install_dir: &str,
) -> (&'static str, Option<String>, Option<Value>) {
    let expanded = command.replace("{install_dir}", install_dir);
    let first = expanded.split_whitespace().next().unwrap_or("");
    if first.is_empty() {
        return ("failed", Some("Launch command is empty.".to_string()), None);
    }
    let path = Path::new(first);
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 == 0 {
                    return (
                        "failed",
                        Some(format!("Launch file is not executable: {first}")),
                        Some(json!({ "path": expanded })),
                    );
                }
            }
            ("ok", None, Some(json!({ "path": expanded })))
        }
        Ok(_) => (
            "failed",
            Some(format!("Launch path is not a file: {first}")),
            Some(json!({ "path": expanded })),
        ),
        Err(err) => (
            "failed",
            Some(format!("Launch file was not found: {err}")),
            Some(json!({ "path": expanded, "error": err.to_string() })),
        ),
    }
}

fn probe_cli(
    command: &str,
    install_dir: &str,
) -> (&'static str, Option<String>, Option<Value>) {
    let command = command.replace("{install_dir}", install_dir);
    let mut parts = command.split_whitespace();
    let first = parts.next().unwrap_or("");
    if first.is_empty() {
        return ("failed", Some("CLI command is empty.".to_string()), None);
    }
    let has_help = parts.any(|argument| argument == "--help" || argument == "-h");
    let executable = if Path::new(first).is_file() {
        PathBuf::from(first)
    } else if let Some(path) = probe::find_in_path(first) {
        path
    } else {
        return (
            "failed",
            Some(format!("Executable '{first}' was not found.")),
            Some(json!({ "error": "executable not found" })),
        );
    };
    let started = Instant::now();
    let mut command = Command::new(&executable);
    if !has_help {
        command.arg("--help");
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return (
                "failed",
                Some(format!("Could not run '{first}': {err}")),
                Some(json!({ "error": err.to_string() })),
            )
        }
    };
    let pid = child.id() as i32;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let latency = started.elapsed().as_millis() as u64;
                let details = json!({ "exitCode": status.code(), "latencyMs": latency });
                return if status.success() {
                    ("ok", None, Some(details))
                } else {
                    (
                        "failed",
                        Some(format!("--help exited with {status}")),
                        Some(json!({ "exitCode": status.code(), "latencyMs": latency })),
                    )
                };
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            _ => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
                #[cfg(not(unix))]
                let _ = child.kill();
                let _ = child.wait();
                return (
                    "failed",
                    Some("--help did not finish within 2 seconds.".to_string()),
                    Some(json!({ "error": "timeout" })),
                );
            }
        }
    }
}

pub fn discover(
    app_data: &Path,
    recipe: &Recipe,
    method: &Method,
    record: &InstalledApp,
) -> Vec<DetectedEntry> {
    if !methods::is_running(method, record, app_data) {
        return Vec::new();
    }
    if !discovery_supported(std::env::consts::OS) {
        return Vec::new();
    }
    let Some(pid) = read_pid(app_data, &record.app_id) else {
        return Vec::new();
    };
    let ports = listening_ports(pid);
    let mut entries = Vec::new();
    for port in ports {
        for (path, kind, confidence) in [
            ("/", "web-ui", "medium"),
            ("/api", "api", "low"),
            ("/health", "health", "high"),
            ("/openapi.json", "openapi", "high"),
            ("/v1/models", "models", "high"),
            ("/api/health", "health", "high"),
            ("/api/openapi.json", "openapi", "high"),
            ("/api/v1/models", "models", "high"),
        ] {
            let url = format!("http://127.0.0.1:{port}{path}");
            if let Some((title, status)) = probe_detected(&url) {
                if status.is_success() {
                    entries.push(DetectedEntry {
                        kind: kind.to_string(),
                        url: url.clone(),
                        title,
                        confidence: confidence.to_string(),
                        evidence: format!("port {port}; GET {path} returned {status}"),
                    });
                }
            }
        }
    }
    entries.sort_by(|left, right| left.url.cmp(&right.url));
    entries.dedup_by(|left, right| left.url == right.url);
    let _ = recipe;
    entries
}

pub fn discovery_supported(os: &str) -> bool {
    os == "linux"
}

fn read_pid(app_data: &Path, app_id: &str) -> Option<i32> {
    std::fs::read_to_string(probe::pid_path(app_data, app_id))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn listening_ports(pid: i32) -> Vec<u16> {
    let mut inodes = HashSet::new();
    if let Ok(entries) = std::fs::read_dir(format!("/proc/{pid}/fd")) {
        for entry in entries.flatten() {
            if let Ok(target) = std::fs::read_link(entry.path()) {
                if let Some(target) = target.to_str() {
                    if let Some(inode) = target
                        .strip_prefix("socket:[")
                        .and_then(|rest| rest.strip_suffix(']'))
                        .and_then(|inode| inode.parse::<u64>().ok())
                    {
                        inodes.insert(inode);
                    }
                }
            }
        }
    }
    if inodes.is_empty() {
        return Vec::new();
    }
    let mut ports = Vec::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            ports.extend(parse_listening_ports(&text, &inodes));
        }
    }
    ports.sort_unstable();
    ports.dedup();
    ports
}

fn parse_listening_ports(text: &str, inodes: &HashSet<u64>) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in text.lines().skip(1) {
        let columns: Vec<&str> = line.split_whitespace().collect();
        if columns.len() < 10 || columns[3] != "0A" {
            continue;
        }
        let inode: u64 = match columns[9].parse() {
            Ok(inode) => inode,
            Err(_) => continue,
        };
        if !inodes.contains(&inode) {
            continue;
        }
        let port = columns[1]
            .split(':')
            .nth(1)
            .and_then(|port| u16::from_str_radix(port, 16).ok());
        if let Some(port) = port {
            ports.push(port);
        }
    }
    ports
}

fn probe_detected(url: &str) -> Option<(Option<String>, reqwest::StatusCode)> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .ok()?;
    let response = client.get(url).send().ok()?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let title = if content_type.contains("text/html") {
        let mut body = Vec::new();
        let _ = response.take(16 * 1024 + 1).read_to_end(&mut body);
        extract_title(&String::from_utf8_lossy(&body))
    } else {
        None
    };
    Some((title, status))
}

fn now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::{classify, extract_title, parse_listening_ports, probe_cli, static_error};
    use crate::catalog::UsageItem;
    use crate::db::UsageCheckRecord;

    fn item(value: &str, kind: &str, is_url: bool) -> UsageItem {
        UsageItem {
            id: "item".to_string(),
            kind: kind.to_string(),
            label: "Item".to_string(),
            value: value.to_string(),
            is_url,
        }
    }

    #[test]
    fn static_check_rejects_invalid_values() {
        assert!(static_error(&item("ftp://example.com", "link", true)).is_some());
        assert!(static_error(&item("demo; rm -rf /", "cli", false)).is_some());
        assert!(static_error(&item("demo --help", "cli", false)).is_none());
        assert!(static_error(&item("http://127.0.0.1:1", "api", true)).is_none());
    }

    #[test]
    fn extracts_html_title() {
        assert_eq!(
            extract_title("<html><head><title>Demo UI</title></head></html>"),
            Some("Demo UI".to_string())
        );
        assert_eq!(extract_title("<html></html>"), None);
    }

    #[test]
    fn parses_listening_ports_from_proc_fixture() {
        let text = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt uid timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 12345 1
   1: 0100007F:1F91 00000000:0000 01 00000000:00000000 00:00000000 00000000 1000 0 12346 1
";
        let ports = parse_listening_ports(text, &HashSet::from([12345]));
        assert_eq!(ports, vec![0x1F90]);
    }

    #[test]
    fn classifies_verification_states() {
        let record = |status: &str, version: &str| UsageCheckRecord {
            version: version.to_string(),
            item_id: "api-base".to_string(),
            status: status.to_string(),
            checked_at: "1".to_string(),
            details_json: "{}".to_string(),
        };
        assert_eq!(classify(None, None, "1.0").0, "unverified");
        assert_eq!(
            classify(Some(&record("ok", "1.0")), None, "1.0").0,
            "verified"
        );
        assert_eq!(
            classify(Some(&record("failed", "1.0")), None, "1.0").0,
            "failed"
        );
        assert_eq!(
            classify(None, Some(&record("ok", "0.9")), "1.0").0,
            "stale"
        );
    }

    #[test]
    fn probes_a_local_http_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 1024];
                let _ = stream.read(&mut buffer);
                let body = "<html><title>Demo UI</title></html>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let url = format!("http://{address}/");
        let (status, message, details) = super::probe_url(&url);
        assert_eq!(status, "ok", "{message:?}");
        assert_eq!(
            details.unwrap().get("title").and_then(|value| value.as_str()),
            Some("Demo UI")
        );
    }

    #[test]
    fn verifies_launch_executables() {
        let directory = std::env::temp_dir().join(format!(
            "easyai-usage-launch-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let binary = directory.join("app");
        fs::write(&binary, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&binary).unwrap().permissions();
            permissions.set_mode(0o644);
            fs::set_permissions(&binary, permissions).unwrap();
        }

        let install_dir = directory.to_string_lossy().to_string();
        let (status, message, _) = super::probe_launch("{install_dir}/app", &install_dir);
        #[cfg(unix)]
        assert_eq!(status, "failed", "{message:?}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&binary).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&binary, permissions).unwrap();
        }
        let (status, message, _) = super::probe_launch("{install_dir}/app", &install_dir);
        assert_eq!(status, "ok", "{message:?}");

        let (status, _, _) = super::probe_launch("{install_dir}/missing", &install_dir);
        assert_eq!(status, "failed");

        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn probes_a_cli_help_command() {
        let directory = std::env::temp_dir().join(format!(
            "easyai-usage-cli-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("demo-tool");
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&script).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&script, permissions).unwrap();
        }
        let command = format!("{} --help", script.display());
        let (status, message, _) = probe_cli(&command, "");
        assert_eq!(status, "ok", "{message:?}");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn probes_a_hanging_cli_with_a_timeout() {
        let directory = std::env::temp_dir().join(format!(
            "easyai-usage-hang-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("slow-tool");
        std::fs::write(&script, "#!/bin/sh\n/bin/sleep 5\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&script).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&script, permissions).unwrap();
        }
        let (status, message, _) = probe_cli(&format!("{} --help", script.display()), "");
        assert_eq!(status, "failed");
        let message = message.unwrap_or_default();
        assert!(message.contains("2 seconds"), "{message}");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn probes_detected_http_endpoints() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 1024];
                let _ = stream.read(&mut buffer);
                let body = "<html><title>Detected UI</title></html>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let (title, status) = super::probe_detected(&format!("http://{address}/")).unwrap();
        assert!(status.is_success());
        assert_eq!(title.as_deref(), Some("Detected UI"));
    }

    #[test]
    fn discovery_support_is_platform_bounded() {
        assert!(super::discovery_supported("linux"));
        assert!(!super::discovery_supported("macos"));
        assert!(!super::discovery_supported("windows"));
    }

    #[test]
    fn discovery_is_empty_when_the_app_is_not_running() {
        let recipe: crate::catalog::Recipe = serde_json::from_value(serde_json::json!({
            "id": "demo",
            "name": "Demo",
            "platforms": ["linux-x86_64"],
            "methods": [{
                "id": "release",
                "kind": "release",
                "source": { "repo": "owner/repo", "asset": { "linux-x86_64": "demo.tar.gz" } },
                "version": { "type": "github-release", "repo": "owner/repo" },
                "uninstall": { "keep_paths": [] }
            }]
        }))
        .unwrap();
        let method = recipe.method("release").unwrap().clone();
        let record = crate::db::InstalledApp {
            app_id: "demo".to_string(),
            version: "1.0.0".to_string(),
            install_dir: "/tmp/demo".to_string(),
            method: "release".to_string(),
            state: "stopped".to_string(),
            managed: true,
            method_id: Some("release".to_string()),
            method_kind: Some("release".to_string()),
            provenance_json: None,
            bom_path: None,
        };
        let directory = std::env::temp_dir().join(format!(
            "easyai-usage-discovery-empty-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        assert!(super::discover(&directory, &recipe, &method, &record).is_empty());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn rejects_a_missing_cli_executable() {
        let (status, _, _) = probe_cli("definitely-not-a-real-easyai-tool --help", "");
        assert_eq!(status, "failed");
    }

    #[test]
    fn runs_checks_stores_results_and_marks_stale_versions() {
        let directory = std::env::temp_dir().join(format!(
            "easyai-usage-integration-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let db_path = directory.join("easyai.db");
        crate::db::init(&db_path).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 1024];
                let _ = stream.read(&mut buffer);
                let body = "<html><title>Demo</title></html>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });

        let script = directory.join("demo-tool");
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&script).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&script, permissions).unwrap();
        }

        let recipe: crate::catalog::Recipe = serde_json::from_value(serde_json::json!({
            "id": "demo",
            "name": "Demo",
            "platforms": ["linux-x86_64"],
            "category": "llm-runtime",
            "usage": {
                "ui_url": format!("http://127.0.0.1:{port}/"),
                "cli": [
                    { "id": "cli-help", "label": "Help", "command": format!("{} --help", script.display()) }
                ]
            },
            "methods": [{
                "id": "release",
                "kind": "release",
                "source": { "repo": "owner/repo", "asset": { "linux-x86_64": "demo.tar.gz" } },
                "version": { "type": "github-release", "repo": "owner/repo" },
                "uninstall": { "keep_paths": [] }
            }]
        }))
        .unwrap();
        recipe.validate().unwrap();
        let method = recipe.method("release").unwrap().clone();
        let mut record = crate::db::InstalledApp {
            app_id: "demo".to_string(),
            version: "1.0.0".to_string(),
            install_dir: directory.join("app").to_string_lossy().to_string(),
            method: "release".to_string(),
            state: "running".to_string(),
            managed: true,
            method_id: Some("release".to_string()),
            method_kind: Some("release".to_string()),
            provenance_json: None,
            bom_path: None,
        };

        let report = super::run_check(&db_path, &recipe, &method, &record, true);
        assert_eq!(report.items.len(), 2);
        assert!(report.items.iter().all(|item| item.state == "verified"));
        assert!(report.items.iter().all(|item| item.details.is_some()));

        let stored = super::report(&db_path, &recipe, &method, &record);
        assert!(stored.items.iter().all(|item| item.state == "verified"));

        record.version = "2.0.0".to_string();
        let stale = super::report(&db_path, &recipe, &method, &record);
        assert!(stale.items.iter().all(|item| item.state == "stale"));

        std::fs::remove_dir_all(&directory).unwrap();
    }
}