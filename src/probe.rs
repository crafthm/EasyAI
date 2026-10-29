use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::catalog::{Method, MethodSource, Recipe};

#[derive(Debug, Clone)]
pub struct HardwareProfile {
    pub os: String,
    pub arch: String,
    pub platform: String,
    pub gpus: Vec<String>,
    pub runtimes: HashSet<String>,
    pub managers: HashSet<String>,
    pub toolchains: HashSet<String>,
    pub ram_mb: u64,
    pub disk_mb: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingInstall {
    pub method_id: Option<String>,
    pub kind: String,
    pub location: String,
    pub version: Option<String>,
    pub managed: bool,
}

pub fn platform_id_for(os: &str, arch: &str) -> Result<String, String> {
    let os = match os {
        "linux" => "linux",
        "macos" => "macos",
        "windows" => "windows",
        other => return Err(format!("Unsupported operating system '{other}'.")),
    };
    let arch = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(format!("Unsupported architecture '{other}'.")),
    };
    Ok(format!("{os}-{arch}"))
}

pub fn current_platform_id() -> Result<String, String> {
    platform_id_for(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn collect(app_data: &Path) -> Result<HardwareProfile, String> {
    let platform = current_platform_id()?;
    let (os, arch) = platform
        .split_once('-')
        .ok_or_else(|| format!("Unsupported platform '{platform}'."))?;
    Ok(HardwareProfile {
        os: os.to_string(),
        arch: arch.to_string(),
        platform,
        gpus: detect_gpus(),
        runtimes: detect_binaries(&[
            "docker", "podman", "python3", "python", "uv", "pipx", "node", "pnpm", "npm",
            "cargo", "go", "java",
        ]),
        managers: detect_binaries(&[
            "brew", "winget", "scoop", "choco", "apt-get", "dnf", "pacman", "zypper", "apk",
            "nix",
        ]),
        toolchains: detect_binaries(&[
            "git", "python3", "python", "node", "cargo", "go", "cmake", "make", "gcc", "java",
            "nix",
        ]),
        ram_mb: total_ram_mb(),
        disk_mb: free_disk_mb(app_data),
    })
}

pub fn asset_for(source: &MethodSource, platform: &str) -> Result<String, String> {
    source
        .asset
        .get(platform)
        .cloned()
        .ok_or_else(|| format!("No release asset is declared for {platform}."))
}

pub fn find_existing(
    recipe: &Recipe,
    profile: &HardwareProfile,
    app_data: &Path,
) -> Vec<ExistingInstall> {
    let mut found = Vec::new();

    for method in &recipe.methods {
        if !method.when.matches_platform(&profile.os, &profile.arch) {
            continue;
        }
        if matches!(method.kind, crate::catalog::MethodKind::Release) {
            let versions = versions_dir(app_data, &recipe.id);
            if let Ok(entries) = std::fs::read_dir(&versions) {
                let mut versions: Vec<String> = entries
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .filter(|name| !name.starts_with('.'))
                    .collect();
                versions.sort();
                if let Some(version) = versions.last() {
                    found.push(ExistingInstall {
                        method_id: Some(method.id.clone()),
                        kind: "release".to_string(),
                        location: versions_dir(app_data, &recipe.id)
                            .join(version)
                            .to_string_lossy()
                            .to_string(),
                        version: Some(version.clone()),
                        managed: true,
                    });
                }
            }
        }
        if matches!(
            method.kind,
            crate::catalog::MethodKind::PackageManager
        ) {
            if let Some(package) = &method.package {
                if let Some(version) = package_version(&package.manager, &package.package) {
                    found.push(ExistingInstall {
                        method_id: Some(method.id.clone()),
                        kind: "package-manager".to_string(),
                        location: package.package.clone(),
                        version: Some(version),
                        managed: false,
                    });
                }
            }
        }
        if matches!(method.kind, crate::catalog::MethodKind::Container) {
            let name = format!("easyai-{}", recipe.id);
            if container_exists(&name) {
                found.push(ExistingInstall {
                    method_id: Some(method.id.clone()),
                    kind: "container".to_string(),
                    location: name,
                    version: None,
                    managed: false,
                });
            }
        }
    }

    let binary = recipe
        .methods
        .iter()
        .find_map(|method| binary_name(recipe, method));
    if let Some(binary) = binary {
        if let Some(path) = find_in_path(&binary) {
            let version = command_version(&path);
            found.push(ExistingInstall {
                method_id: None,
                kind: "binary".to_string(),
                location: path.to_string_lossy().to_string(),
                version,
                managed: false,
            });
        }
    }

    found.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.location.cmp(&right.location))
    });
    found.dedup_by(|left, right| left.kind == right.kind && left.location == right.location);
    found
}

pub fn binary_name(_recipe: &Recipe, method: &Method) -> Option<String> {
    if let Some(language) = &method.language {
        if let Some(executable) = &language.executable {
            return Some(executable.clone());
        }
    }
    if let Some(package) = &method.package {
        return Some(package.package.clone());
    }
    if let Some(service) = &method.service {
        if let Some(first) = service.command.first() {
            if !first.contains('{') {
                return Path::new(first)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string);
            }
        }
    }
    None
}

pub fn apps_dir(app_data: &Path) -> PathBuf {
    app_data.join("apps")
}

pub fn app_dir(app_data: &Path, app_id: &str) -> PathBuf {
    apps_dir(app_data).join(app_id)
}

pub fn versions_dir(app_data: &Path, app_id: &str) -> PathBuf {
    app_dir(app_data, app_id).join("versions")
}

pub fn version_dir(app_data: &Path, app_id: &str, version: &str) -> PathBuf {
    versions_dir(app_data, app_id).join(version)
}

pub fn downloads_dir(app_data: &Path) -> PathBuf {
    app_data.join("downloads")
}

pub fn logs_dir(app_data: &Path) -> PathBuf {
    app_data.join("logs")
}

pub fn run_dir(app_data: &Path) -> PathBuf {
    app_data.join("run")
}

pub fn pid_path(app_data: &Path, app_id: &str) -> PathBuf {
    run_dir(app_data).join(format!("{app_id}.pid"))
}

pub fn bom_path(app_data: &Path, app_id: &str) -> PathBuf {
    app_data.join("bom").join(format!("{app_id}.json"))
}

fn detect_gpus() -> Vec<String> {
    let nvidia = Command::new("nvidia-smi")
        .arg("--query-gpu=name")
        .arg("--format=csv,noheader")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let rocm = Command::new("rocm-smi")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    gpu_set(nvidia, rocm, std::env::consts::OS == "macos")
}

fn gpu_set(nvidia: bool, rocm: bool, apple: bool) -> Vec<String> {
    let mut gpus = Vec::new();
    if nvidia {
        gpus.push("cuda".to_string());
    }
    if rocm {
        gpus.push("rocm".to_string());
    }
    if apple {
        gpus.push("metal".to_string());
    }
    gpus.push("cpu".to_string());
    gpus
}

fn detect_binaries(names: &[&str]) -> HashSet<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    detect_binaries_with(&path, names)
}

pub fn detect_binaries_with(path: &std::ffi::OsStr, names: &[&str]) -> HashSet<String> {
    names
        .iter()
        .filter(|name| find_in_path_with(path, name).is_some())
        .map(|name| name.to_string())
        .collect()
}

pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    find_in_path_with(&path, name)
}

pub fn find_in_path_with(path: &std::ffi::OsStr, name: &str) -> Option<PathBuf> {
    for directory in std::env::split_paths(path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn command_version(binary: &Path) -> Option<String> {
    let mut child = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let output = child.wait_with_output().ok()?;
                let text = String::from_utf8_lossy(&output.stdout);
                return text.lines().next().map(|line| line.trim().to_string());
            }
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn package_version(manager: &str, package: &str) -> Option<String> {
    let output = match manager {
        "brew" => Command::new("brew")
            .args(["list", "--versions", package])
            .output()
            .ok()?,
        "winget" => Command::new("winget")
            .args(["list", "--id", package, "--exact"])
            .output()
            .ok()?,
        "apt" | "apt-get" => Command::new("dpkg-query")
            .args(["-W", "-f=${Version}", package])
            .output()
            .ok()?,
        "dnf" | "rpm" => Command::new("rpm")
            .args(["-q", "--qf", "%{VERSION}", package])
            .output()
            .ok()?,
        "pacman" => Command::new("pacman").args(["-Q", package]).output().ok()?,
        _ => return None,
    };
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_package_version(manager, package, &text)
}

fn parse_package_version(manager: &str, package: &str, output: &str) -> Option<String> {
    let output = output.trim();
    if output.is_empty() {
        return None;
    }
    match manager {
        "brew" => output
            .split_whitespace()
            .nth(1)
            .map(str::to_string)
            .or_else(|| Some(output.to_string())),
        "winget" => output.lines().find_map(|line| {
            if line.contains(package) {
                line.split_whitespace()
                    .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()))
                    .map(str::to_string)
            } else {
                None
            }
        }),
        "pacman" => output
            .split_whitespace()
            .nth(1)
            .map(str::to_string),
        _ => Some(output.to_string()),
    }
}

fn container_exists(name: &str) -> bool {
    let output = Command::new("docker")
        .args(["ps", "-a", "--filter", &format!("name={name}"), "--format", "{{.Names}}"])
        .output();
    output
        .map(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line.trim() == name)
        })
        .unwrap_or(false)
}

fn total_ram_mb() -> u64 {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return 0;
    };
    text.lines()
        .find_map(|line| {
            let rest = line.strip_prefix("MemTotal:")?;
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            Some(kb / 1024)
        })
        .unwrap_or(0)
}

fn free_disk_mb(path: &Path) -> u64 {
    use std::ffi::CString;
    let Ok(path) = CString::new(path.to_string_lossy().as_bytes()) else {
        return 0;
    };
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::statvfs(path.as_ptr(), &mut stat) };
    if result != 0 {
        return 0;
    }
    (stat.f_bavail as u64)
        .saturating_mul(stat.f_frsize as u64)
        .saturating_div(1024 * 1024)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::{gpu_set, parse_package_version, platform_id_for, HardwareProfile};
    use crate::catalog::Recipe;

    #[test]
    fn maps_supported_platforms() {
        assert_eq!(platform_id_for("linux", "x86_64").unwrap(), "linux-x86_64");
        assert_eq!(platform_id_for("linux", "aarch64").unwrap(), "linux-aarch64");
        assert_eq!(platform_id_for("macos", "aarch64").unwrap(), "macos-aarch64");
    }

    #[test]
    fn rejects_unsupported_platforms() {
        assert!(platform_id_for("freebsd", "x86_64").is_err());
        assert!(platform_id_for("linux", "riscv64").is_err());
    }

    #[test]
    fn gpu_sets_include_cpu_fallback() {
        assert_eq!(gpu_set(true, false, false), vec!["cuda", "cpu"]);
        assert_eq!(gpu_set(false, true, false), vec!["rocm", "cpu"]);
        assert_eq!(gpu_set(false, false, true), vec!["metal", "cpu"]);
        assert_eq!(gpu_set(false, false, false), vec!["cpu"]);
    }

    #[test]
    fn parses_package_versions() {
        assert_eq!(
            parse_package_version("brew", "demo", "demo 1.2.3"),
            Some("1.2.3".to_string())
        );
        assert_eq!(
            parse_package_version("pacman", "demo", "demo 1.2.3-1"),
            Some("1.2.3-1".to_string())
        );
    }

    #[test]
    fn detects_binaries_by_name() {
        let found = super::detect_binaries(&["sh", "definitely-not-a-real-binary"]);
        assert!(found.contains("sh"));
        assert!(!found.contains("definitely-not-a-real-binary"));
    }

    #[test]
    fn finds_an_existing_release_install() {
        let directory = std::env::temp_dir().join(format!(
            "easyai-existing-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let version_dir = directory.join("apps/demo/versions/1.0.0");
        std::fs::create_dir_all(&version_dir).unwrap();
        let recipe: Recipe = serde_json::from_value(json!({
            "id": "demo",
            "name": "Demo",
            "platforms": ["linux-x86_64"],
            "methods": [{
                "id": "release",
                "kind": "release",
                "when": { "os": ["linux"], "arch": ["x86_64"] },
                "source": { "repo": "owner/repo", "asset": { "linux-x86_64": "demo.tar.gz" } },
                "version": { "type": "github-release", "repo": "owner/repo" },
                "uninstall": { "keep_paths": [] }
            }]
        }))
        .unwrap();
        let profile = HardwareProfile {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            platform: "linux-x86_64".to_string(),
            gpus: vec!["cpu".to_string()],
            runtimes: HashSet::new(),
            managers: HashSet::new(),
            toolchains: HashSet::new(),
            ram_mb: 8192,
            disk_mb: 1024,
        };
        let found = super::find_existing(&recipe, &profile, &directory);
        assert!(found.iter().any(|install| {
            install.kind == "release" && install.version.as_deref() == Some("1.0.0")
        }));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn finds_binaries_in_a_given_path() {
        let directory = std::env::temp_dir().join(format!(
            "easyai-path-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("demo-tool"), b"#!/bin/sh\n").unwrap();
        let found = super::find_in_path_with(directory.as_os_str(), "demo-tool");
        assert!(found.is_some());
        assert!(super::find_in_path_with(directory.as_os_str(), "missing-tool").is_none());
        let detected = super::detect_binaries_with(
            directory.as_os_str(),
            &["demo-tool", "missing-tool"],
        );
        assert!(detected.contains("demo-tool"));
        assert!(!detected.contains("missing-tool"));
        std::fs::remove_dir_all(&directory).unwrap();
    }
}