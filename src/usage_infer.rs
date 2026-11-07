use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::catalog::{Category, CliUsage, LaunchUsage};
use crate::github::{ReleaseAsset, ReleaseInfo};

const MAX_INSPECT_BYTES: u64 = 200 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 64 * 1024;
const MAX_TEXT_FILES: usize = 32;

#[derive(Debug, Clone)]
pub struct Inference {
    pub category: Option<Category>,
    pub launch: Option<LaunchUsage>,
    pub cli: Vec<CliUsage>,
    pub warnings: Vec<String>,
    pub source: String,
    pub confidence: String,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub mode: Option<u32>,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Inventory {
    pub entries: Vec<Entry>,
    pub texts: BTreeMap<String, String>,
}

pub fn infer_release(
    owner: &str,
    repo_name: &str,
    description: Option<&str>,
    release: &ReleaseInfo,
    platform: &str,
) -> Inference {
    let Some(asset) = select_asset(release, platform) else {
        return empty_inference("No inspectable release archive was found for this machine.");
    };
    if asset.size > MAX_INSPECT_BYTES {
        return empty_inference("The release archive is too large to inspect.");
    }
    let url = format!(
        "https://github.com/{owner}/{repo_name}/releases/download/{}/{}",
        release.tag_name, asset.name
    );
    let path = temp_path(owner, &release.tag_name, &asset.name);
    let result = download(&url, &path)
        .and_then(|()| read_inventory(&path, &asset.name))
        .map(|inventory| infer_inventory(repo_name, description, &inventory));
    let _ = fs::remove_file(&path);
    match result {
        Ok(inference) => inference,
        Err(err) => empty_inference(&format!("Release archive inspection failed: {err}")),
    }
}

pub fn infer_inventory(
    app_name: &str,
    description: Option<&str>,
    inventory: &Inventory,
) -> Inference {
    let (launch, launch_confidence) = match pick_launch_candidate(inventory, app_name) {
        Some((path, confidence)) => (
            Some(LaunchUsage {
                command: format!("{{install_dir}}/{path}"),
                source: Some("artifact".to_string()),
                confidence: Some(confidence.to_string()),
            }),
            confidence,
        ),
        None => (None, "low"),
    };
    let cli = pick_cli_candidate(inventory, app_name)
        .map(|path| {
            vec![CliUsage {
                id: Some("cli-help".to_string()),
                label: "CLI help".to_string(),
                command: format!("{{install_dir}}/{path} --help"),
            }]
        })
        .unwrap_or_default();
    let category = infer_category(description, inventory, launch.is_some());
    let mut warnings = Vec::new();
    if launch.is_none() {
        warnings.push(
            "No launch executable was found in the release archive.".to_string(),
        );
    }
    if category.is_none() {
        warnings.push("The application category could not be inferred.".to_string());
    }
    Inference {
        category,
        launch,
        cli,
        warnings,
        source: "artifact".to_string(),
        confidence: if category.is_some() {
            launch_confidence.to_string()
        } else {
            "low".to_string()
        },
    }
}

fn empty_inference(warning: &str) -> Inference {
    Inference {
        category: None,
        launch: None,
        cli: Vec::new(),
        warnings: vec![warning.to_string()],
        source: "artifact".to_string(),
        confidence: "low".to_string(),
    }
}

fn infer_category(
    description: Option<&str>,
    inventory: &Inventory,
    has_launch: bool,
) -> Option<Category> {
    let description = description.unwrap_or("").to_ascii_lowercase();
    let names = inventory
        .entries
        .iter()
        .map(|entry| entry.path.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    let texts = inventory
        .texts
        .values()
        .map(|text| text.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("\n");

    if names.contains(".app/")
        || names.ends_with(".app")
        || names.contains(".desktop")
        || names.ends_with(".exe")
        || description.contains("desktop")
        || description.contains("workspace")
        || description.contains("video editor")
    {
        return Some(Category::DesktopApp);
    }
    if texts.contains("expose ")
        || texts.contains("ports:")
        || texts.contains("http://localhost")
        || texts.contains("http://127.0.0.1")
    {
        return Some(Category::WebUi);
    }
    if description.contains("comfy")
        || description.contains("diffusion")
        || description.contains("image generation")
        || texts.contains("comfy")
    {
        return Some(Category::ImageGeneration);
    }
    if description.contains("ollama")
        || description.contains("llama.cpp")
        || description.contains("vllm")
        || description.contains("language model")
        || texts.contains("/v1/models")
    {
        return Some(Category::LlmRuntime);
    }
    if has_launch {
        return Some(Category::CliTool);
    }
    None
}

fn pick_launch_candidate(
    inventory: &Inventory,
    app_name: &str,
) -> Option<(String, &'static str)> {
    let wanted = app_name.trim().to_ascii_lowercase();
    let mut best: Option<(u32, u64, String, &'static str)> = None;

    for entry in &inventory.entries {
        if entry.is_dir || entry.size == 0 {
            continue;
        }
        let lower = entry.path.to_ascii_lowercase();
        if is_excluded(&lower) || is_library(&lower) {
            continue;
        }
        let stem = Path::new(&lower)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let (name_score, confidence) = if !wanted.is_empty() && stem == wanted {
            (0u32, "high")
        } else if !wanted.is_empty() && (stem.contains(&wanted) || wanted.contains(stem)) {
            (1u32, "medium")
        } else if is_executable_entry(entry, &lower) {
            (2u32, "low")
        } else {
            continue;
        };
        let score = name_score * 1000
            + entry.path.matches('/').count() as u32 * 10
            + if is_executable_entry(entry, &lower) { 0 } else { 5 };
        let candidate = (score, entry.size, entry.path.clone(), confidence);
        if best
            .as_ref()
            .is_none_or(|(best_score, best_size, _, _)| {
                score < *best_score || (score == *best_score && entry.size > *best_size)
            })
        {
            best = Some(candidate);
        }
    }

    best.map(|(_, _, path, confidence)| (path, confidence))
}

/// The bundled CLI usually ships in a `cli/` directory next to the app binary.
fn pick_cli_candidate(inventory: &Inventory, app_name: &str) -> Option<String> {
    let wanted = app_name.trim().to_ascii_lowercase();
    let mut best: Option<(u32, String)> = None;

    for entry in &inventory.entries {
        if entry.is_dir || entry.size == 0 {
            continue;
        }
        let lower = entry.path.to_ascii_lowercase();
        if !(lower.starts_with("cli/") || lower.contains("/cli/")) {
            continue;
        }
        if is_excluded(&lower) || is_library(&lower) {
            continue;
        }
        let stem = Path::new(&lower)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let matched = !wanted.is_empty()
            && (stem == wanted || stem.contains(&wanted) || wanted.contains(stem));
        if !matched && !is_executable_entry(entry, &lower) {
            continue;
        }
        let score = if matched { 0 } else { 100 }
            + entry.path.matches('/').count() as u32;
        if best
            .as_ref()
            .is_none_or(|(best_score, _)| score < *best_score)
        {
            best = Some((score, entry.path.clone()));
        }
    }

    best.map(|(_, path)| path)
}

fn is_excluded(path: &str) -> bool {
    const EXCLUDED: [&str; 18] = [
        "node_modules/",
        "/docs/",
        "/doc/",
        "/examples/",
        "/example/",
        "/tests/",
        "/test/",
        "/templates/",
        "/samples/",
        "/vendor/",
        "/third_party/",
        "/third-party/",
        "uninstall",
        "uninstaller",
        "readme",
        "license",
        "changelog",
        "crash",
    ];
    EXCLUDED.iter().any(|part| path.contains(part))
}

fn is_library(path: &str) -> bool {
    const LIBRARY_EXTENSIONS: [&str; 18] = [
        ".so", ".dylib", ".dll", ".a", ".lib", ".o", ".py", ".pyc", ".js", ".mjs",
        ".ts", ".json", ".md", ".txt", ".yml", ".yaml", ".gguf", ".safetensors",
    ];
    LIBRARY_EXTENSIONS
        .iter()
        .any(|extension| path.ends_with(extension))
}

fn is_executable_entry(entry: &Entry, lower_path: &str) -> bool {
    entry.mode.is_some_and(|mode| mode & 0o111 != 0)
        || lower_path.ends_with(".exe")
        || lower_path.ends_with(".bat")
        || lower_path.ends_with(".cmd")
        || lower_path.ends_with(".ps1")
        || lower_path.ends_with(".appimage")
        || lower_path.contains("/contents/macos/")
}

fn select_asset<'a>(release: &'a ReleaseInfo, platform: &str) -> Option<&'a ReleaseAsset> {
    let (os, arch) = platform.split_once('-').unwrap_or(("", ""));
    let os_tokens: &[&str] = match os {
        "linux" => &["linux"],
        "macos" => &["macos", "darwin", "osx"],
        "windows" => &["windows", "win64", "win32"],
        _ => &[],
    };
    let arch_tokens: &[&str] = match arch {
        "x86_64" => &["x86_64", "amd64", "x64"],
        "aarch64" => &["aarch64", "arm64"],
        _ => &[],
    };
    release
        .assets
        .iter()
        .filter_map(|asset| {
            let name = asset.name.to_ascii_lowercase();
            let rank = archive_rank(&name)?;
            let other_os = ["linux", "macos", "darwin", "windows", "win64", "win32"]
                .iter()
                .any(|token| name.contains(token) && !os_tokens.contains(token));
            if other_os {
                return None;
            }
            let mut score = rank as i32 * 100;
            if os_tokens.iter().any(|token| name.contains(token)) {
                score += 1000;
            }
            if arch_tokens.iter().any(|token| name.contains(token)) {
                score += 500;
            }
            Some((score, asset))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, asset)| asset)
}

fn archive_rank(name: &str) -> Option<u8> {
    if name.ends_with(".zip") {
        Some(5)
    } else if name.ends_with(".tar.zst") {
        Some(4)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Some(3)
    } else if name.ends_with(".tar") {
        Some(2)
    } else {
        None
    }
}

fn temp_path(owner: &str, tag: &str, asset: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "easyai-inspect-{owner}-{}-{nonce}-{asset}",
        tag.replace('/', "_")
    ))
}

fn download(url: &str, destination: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("EasyAI/0.1")
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|err| format!("HTTP client: {err}"))?;
    let mut response = client
        .get(url)
        .send()
        .map_err(|err| format!("download failed: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("release server returned {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_INSPECT_BYTES)
    {
        return Err("archive is too large to inspect".to_string());
    }
    let mut file = File::create(destination)
        .map_err(|err| format!("create temporary file: {err}"))?;
    let mut written = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|err| format!("read archive: {err}"))?;
        if read == 0 {
            break;
        }
        written += read as u64;
        if written > MAX_INSPECT_BYTES {
            return Err("archive is too large to inspect".to_string());
        }
        file.write_all(&buffer[..read])
            .map_err(|err| format!("write archive: {err}"))?;
    }
    Ok(())
}

fn read_inventory(path: &Path, asset_name: &str) -> Result<Inventory, String> {
    let lower = asset_name.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        read_zip(path)
    } else if lower.ends_with(".tar.zst") {
        let file = File::open(path).map_err(|err| format!("open archive: {err}"))?;
        let decoder = zstd::stream::read::Decoder::new(file)
            .map_err(|err| format!("read zstd archive: {err}"))?;
        read_tar(decoder)
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        let file = File::open(path).map_err(|err| format!("open archive: {err}"))?;
        let decoder = flate2::read::GzDecoder::new(file);
        read_tar(decoder)
    } else if lower.ends_with(".tar") {
        let file = File::open(path).map_err(|err| format!("open archive: {err}"))?;
        read_tar(file)
    } else {
        Err("unsupported archive format".to_string())
    }
}

fn read_zip(path: &Path) -> Result<Inventory, String> {
    let file = File::open(path).map_err(|err| format!("open archive: {err}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("read zip archive: {err}"))?;
    let mut inventory = Inventory::default();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| format!("read zip entry: {err}"))?;
        let path = entry.name().to_string();
        let is_dir = entry.is_dir();
        let size = entry.size();
        let mode = entry.unix_mode();
        if !is_dir && size <= MAX_TEXT_BYTES && wants_text(&path) {
            let mut text = Vec::new();
            let _ = entry.take(MAX_TEXT_BYTES).read_to_end(&mut text);
            if inventory.texts.len() < MAX_TEXT_FILES {
                inventory
                    .texts
                    .insert(path.clone(), String::from_utf8_lossy(&text).to_string());
            }
        }
        inventory.entries.push(Entry {
            path,
            size,
            mode,
            is_dir,
        });
    }
    Ok(inventory)
}

fn read_tar<R: Read>(reader: R) -> Result<Inventory, String> {
    let mut archive = tar::Archive::new(reader);
    let mut inventory = Inventory::default();
    let entries = archive
        .entries()
        .map_err(|err| format!("read tar archive: {err}"))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read tar entry: {err}"))?;
        let path = entry
            .path()
            .map_err(|err| format!("read tar path: {err}"))?
            .to_string_lossy()
            .trim_start_matches("./")
            .to_string();
        let header = entry.header();
        let is_dir = header.entry_type().is_dir();
        let size = header.size().unwrap_or(0);
        let mode = header.mode().ok();
        if !is_dir && size <= MAX_TEXT_BYTES && wants_text(&path) {
            let mut text = Vec::new();
            let _ = entry.take(MAX_TEXT_BYTES).read_to_end(&mut text);
            if inventory.texts.len() < MAX_TEXT_FILES {
                inventory
                    .texts
                    .insert(path.clone(), String::from_utf8_lossy(&text).to_string());
            }
        }
        inventory.entries.push(Entry {
            path,
            size,
            mode,
            is_dir,
        });
    }
    Ok(inventory)
}

fn wants_text(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or("");
    base.starts_with("readme")
        || base == "dockerfile"
        || base == "containerfile"
        || base.starts_with("compose.")
        || base.ends_with(".desktop")
        || base == "info.plist"
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::{infer_inventory, pick_launch_candidate, Entry, Inventory};
    use crate::catalog::Category;

    fn entry(path: &str, size: u64, mode: Option<u32>) -> Entry {
        Entry {
            path: path.to_string(),
            size,
            mode,
            is_dir: false,
        }
    }

    #[test]
    fn infers_a_desktop_launch_from_an_archive() {
        let inventory = Inventory {
            entries: vec![
                entry("BeefTV-linux/BeefTV", 42_000_000, Some(0o755)),
                entry("BeefTV-linux/node_modules/helper.js", 100, None),
                entry("BeefTV-linux/README.md", 100, None),
            ],
            texts: Default::default(),
        };
        let inference =
            infer_inventory("BeefTV", Some("AI Native Video workspace"), &inventory);
        assert_eq!(inference.category, Some(Category::DesktopApp));
        let launch = inference.launch.unwrap();
        assert_eq!(launch.command, "{install_dir}/BeefTV-linux/BeefTV");
        assert_eq!(launch.confidence.as_deref(), Some("high"));
    }

    #[test]
    fn infers_a_bundled_cli_from_the_cli_directory() {
        let inventory = Inventory {
            entries: vec![
                entry("BeefTV-linux/BeefTV", 42_000_000, Some(0o755)),
                entry("BeefTV-linux/cli/beeftv", 1_000_000, Some(0o755)),
            ],
            texts: Default::default(),
        };
        let inference = infer_inventory("BeefTV", None, &inventory);
        assert_eq!(inference.cli.len(), 1);
        assert_eq!(
            inference.cli[0].command,
            "{install_dir}/BeefTV-linux/cli/beeftv --help"
        );
    }

    #[test]
    fn excludes_helpers_and_libraries() {
        let inventory = Inventory {
            entries: vec![
                entry("app/lib/libdemo.so", 100, Some(0o755)),
                entry("app/tools/uninstall", 100, Some(0o755)),
                entry("app/bin/demo", 100, Some(0o755)),
            ],
            texts: Default::default(),
        };
        assert_eq!(
            pick_launch_candidate(&inventory, "demo").unwrap().0,
            "app/bin/demo"
        );
    }

    #[test]
    fn selects_the_current_platform_asset() {
        let release = crate::github::ReleaseInfo {
            tag_name: "v1".to_string(),
            assets: vec![
                crate::github::ReleaseAsset {
                    name: "demo-linux-amd64.zip".to_string(),
                    size: 1,
                },
                crate::github::ReleaseAsset {
                    name: "demo-darwin-arm64.zip".to_string(),
                    size: 1,
                },
            ],
        };
        let selected = super::select_asset(&release, "linux-x86_64").unwrap();
        assert_eq!(selected.name, "demo-linux-amd64.zip");
        let selected = super::select_asset(&release, "macos-aarch64").unwrap();
        assert_eq!(selected.name, "demo-darwin-arm64.zip");
    }

    #[test]
    fn oversized_assets_are_not_inspected() {
        let release = crate::github::ReleaseInfo {
            tag_name: "v1".to_string(),
            assets: vec![crate::github::ReleaseAsset {
                name: "demo-linux-amd64.zip".to_string(),
                size: super::MAX_INSPECT_BYTES + 1,
            }],
        };
        let inference =
            super::infer_release("owner", "demo", None, &release, "linux-x86_64");
        assert!(inference.launch.is_none());
        assert!(inference
            .warnings
            .iter()
            .any(|warning| warning.contains("too large")));
    }

    #[test]
    fn infers_a_macos_app_launch() {
        let inventory = Inventory {
            entries: vec![entry(
                "Demo.app/Contents/MacOS/Demo",
                1_000_000,
                Some(0o755),
            )],
            texts: Default::default(),
        };
        let inference = infer_inventory("Demo", None, &inventory);
        assert_eq!(inference.category, Some(Category::DesktopApp));
        assert_eq!(
            inference.launch.unwrap().command,
            "{install_dir}/Demo.app/Contents/MacOS/Demo"
        );
    }

    #[test]
    fn infers_runtime_image_and_cli_categories() {
        let empty = Inventory::default();
        assert_eq!(
            infer_inventory("demo", Some("Ollama runtime"), &empty).category,
            Some(Category::LlmRuntime)
        );
        assert_eq!(
            infer_inventory("demo", Some("ComfyUI image generation"), &empty).category,
            Some(Category::ImageGeneration)
        );
        let cli = Inventory {
            entries: vec![entry("demo", 10, Some(0o755))],
            texts: Default::default(),
        };
        assert_eq!(
            infer_inventory("demo", None, &cli).category,
            Some(Category::CliTool)
        );
        assert_eq!(infer_inventory("demo", None, &empty).category, None);
    }

    #[test]
    fn infers_web_ui_from_exposed_ports() {
        let mut texts = std::collections::BTreeMap::new();
        texts.insert(
            "Dockerfile".to_string(),
            "FROM x\nEXPOSE 8080\n".to_string(),
        );
        let inventory = Inventory {
            entries: vec![entry("server", 100, Some(0o755))],
            texts,
        };
        let inference = infer_inventory("demo", None, &inventory);
        assert_eq!(inference.category, Some(Category::WebUi));
    }

    #[test]
    fn unsupported_assets_are_skipped() {
        let release = crate::github::ReleaseInfo {
            tag_name: "v1".to_string(),
            assets: vec![crate::github::ReleaseAsset {
                name: "demo-linux-amd64.dmg".to_string(),
                size: 10,
            }],
        };
        let inference =
            super::infer_release("owner", "demo", None, &release, "linux-x86_64");
        assert!(inference.launch.is_none());
        assert!(inference
            .warnings
            .iter()
            .any(|warning| warning.contains("No inspectable")));
    }

    #[test]
    fn reads_a_tar_inventory() {
        let directory = std::env::temp_dir().join(format!(
            "easyai-infer-tar-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("demo.tar.gz");
        {
            let file = std::fs::File::create(&path).unwrap();
            let encoder =
                flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_path("demo/bin/demo").unwrap();
            header.set_size(9);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "demo/bin/demo", &b"#!/bin/sh"[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let inventory = super::read_inventory(&path, "demo.tar.gz").unwrap();
        assert!(inventory.entries.iter().any(|entry| {
            entry.path == "demo/bin/demo" && entry.mode.is_some()
        }));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn reads_a_zip_inventory() {
        let directory = std::env::temp_dir().join(format!(
            "easyai-infer-zip-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("demo.zip");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("demo/bin/demo", options).unwrap();
            writer.write_all(b"#!/bin/sh\n").unwrap();
            writer.start_file("README.md", options).unwrap();
            writer.write_all(b"# Demo\n").unwrap();
            writer.finish().unwrap();
        }
        let inventory = super::read_zip(&path).unwrap();
        assert!(inventory.entries.iter().any(|entry| entry.path == "demo/bin/demo"));
        assert!(inventory.texts.contains_key("README.md"));
        std::fs::remove_dir_all(&directory).unwrap();
    }
}