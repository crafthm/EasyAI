use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use tauri::State;

use crate::db::{self, RepoInfo};
use crate::AppState;

#[derive(Deserialize)]
struct GithubRepo {
    id: i64,
    full_name: String,
    name: String,
    html_url: String,
    owner: GithubOwner,
    description: Option<String>,
    language: Option<String>,
    stargazers_count: i64,
    forks_count: i64,
    open_issues_count: i64,
    default_branch: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    license: Option<GithubLicense>,
    homepage: Option<String>,
    clone_url: String,
    archived: bool,
    private: bool,
    created_at: Option<String>,
    updated_at: Option<String>,
    pushed_at: Option<String>,
}

#[derive(Deserialize)]
struct GithubOwner {
    login: String,
}

#[derive(Deserialize)]
struct GithubLicense {
    spdx_id: Option<String>,
}

#[tauri::command]
pub async fn add_repo(state: State<'_, AppState>, url: String) -> Result<String, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || add_repo_blocking(&db_path, &url))
        .await
        .map_err(|err| format!("Failed to add repository: {err}"))?
}

fn add_repo_blocking(db_path: &Path, url: &str) -> Result<String, String> {
    let (owner, name) = parse_repo_url(url)?;
    let destination = download_dir()?.join(&owner).join(&name);
    if destination.join("README.md").exists() {
        return Err(format!(
            "This repository has already been added at {}.",
            destination.display()
        ));
    }

    let repo = fetch_repo(&owner, &name)?;
    let readme = fetch_readme(&owner, &name)?;

    fs::create_dir_all(&destination)
        .map_err(|err| format!("Failed to create {}: {err}", destination.display()))?;
    let readme_path = destination.join("README.md");
    fs::write(&readme_path, readme)
        .map_err(|err| format!("Failed to write {}: {err}", readme_path.display()))?;

    db::upsert_repo(
        db_path,
        &RepoInfo {
            id: repo.id,
            full_name: repo.full_name.clone(),
            owner: repo.owner.login,
            name: repo.name,
            html_url: repo.html_url,
            description: repo.description,
            language: repo.language,
            stargazers_count: repo.stargazers_count,
            forks_count: repo.forks_count,
            open_issues_count: repo.open_issues_count,
            default_branch: repo.default_branch,
            topics: serde_json::to_string(&repo.topics).unwrap_or_else(|_| "[]".into()),
            license: repo.license.and_then(|license| license.spdx_id),
            homepage: repo.homepage,
            clone_url: repo.clone_url,
            archived: repo.archived,
            private: repo.private,
            created_at: repo.created_at,
            updated_at: repo.updated_at,
            pushed_at: repo.pushed_at,
        },
    )
    .map_err(|err| format!("Failed to save repository: {err}"))?;

    Ok(repo.full_name)
}

/// Accepts `https://github.com/{owner}/{repo}` with an optional `.git` suffix
/// or trailing slash, and returns `(owner, repo)`.
fn parse_repo_url(input: &str) -> Result<(String, String), String> {
    let invalid = || {
        "Enter a valid GitHub repository URL, e.g. https://github.com/owner/repo.".to_string()
    };

    let url = reqwest::Url::parse(input.trim()).map_err(|_| invalid())?;
    if url.scheme() != "https" || url.host_str() != Some("github.com") {
        return Err(invalid());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid());
    }

    let segments: Vec<&str> = url
        .path_segments()
        .ok_or_else(invalid)?
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() != 2 {
        return Err(invalid());
    }

    let owner = segments[0];
    let name = segments[1].strip_suffix(".git").unwrap_or(segments[1]);
    if !is_valid_name(owner) || !is_valid_name(name) {
        return Err(invalid());
    }

    Ok((owner.to_string(), name.to_string()))
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

/// Repositories are saved next to the running executable and are not configurable.
fn download_dir() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|err| format!("Failed to locate the application: {err}"))?;
    exe.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Failed to locate the application directory.".to_string())
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("EasyAI/0.1")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| format!("Failed to create HTTP client: {err}"))
}

/// Metadata used to create a draft catalog entry when a repository does not
/// publish an EasyAI recipe.
pub struct RepoSummary {
    pub name: String,
    pub html_url: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub logo: Option<String>,
}

#[derive(Deserialize)]
struct GithubTree {
    #[serde(default)]
    tree: Vec<GithubTreeEntry>,
}

#[derive(Deserialize)]
struct GithubTreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    #[serde(default)]
    pub size: u64,
}

/// Latest published (non-prerelease) release, or `None` when the repository
/// has not published one.
pub fn latest_release(owner: &str, name: &str) -> Result<Option<ReleaseInfo>, String> {
    let response = http_client()?
        .get(format!(
            "https://api.github.com/repos/{owner}/{name}/releases/latest"
        ))
        .send()
        .map_err(|err| format!("Could not reach GitHub: {err}"))?;

    let status = response.status();
    if status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(match status.as_u16() {
            403 => "GitHub API rate limit exceeded. Try again later.".to_string(),
            _ => format!("GitHub API returned {status}."),
        });
    }

    response
        .json::<ReleaseInfo>()
        .map(Some)
        .map_err(|err| format!("Invalid release metadata from GitHub: {err}"))
}

pub fn repo_summary(owner: &str, name: &str) -> Result<RepoSummary, String> {
    let repo = fetch_repo(owner, name)?;
    let logo = repo
        .default_branch
        .as_deref()
        .and_then(|branch| find_logo(owner, name, branch).ok().flatten());
    Ok(RepoSummary {
        name: repo.name,
        html_url: repo.html_url,
        description: repo.description,
        homepage: repo
            .homepage
            .filter(|url| url.starts_with("http://") || url.starts_with("https://")),
        logo,
    })
}

/// How many candidates are probed before giving up. Keeps the import preview
/// bounded when a repository ships many look-alike images.
const MAX_LOGO_CANDIDATES: usize = 4;
/// Smallest accepted raster edge, so tiles stay sharp on HiDPI screens.
const MIN_LOGO_EDGE: u32 = 96;

/// Finds the project's own logo: first the icon a framework manifest declares,
/// then the best-scoring image in the repository, each verified by actually
/// downloading it. Falls back to the owner avatar when nothing qualifies.
fn find_logo(owner: &str, name: &str, branch: &str) -> Result<Option<String>, String> {
    let Some(tree) = fetch_tree(owner, name, branch) else {
        return Ok(None);
    };

    let mut candidates = declared_icons(owner, name, branch, &tree)
        .into_iter()
        .map(|path| (0u32, path))
        .collect::<Vec<_>>();
    candidates.extend(
        tree.iter()
            .filter(|entry| entry.kind == "blob")
            .filter_map(|entry| logo_score(&entry.path).map(|score| (score, entry.path.clone())))
            .collect::<Vec<_>>(),
    );
    candidates.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.len().cmp(&right.1.len()))
    });
    let mut seen = Vec::new();
    candidates.retain(|(_, path)| {
        if seen.contains(path) {
            return false;
        }
        seen.push(path.clone());
        true
    });

    for (_, path) in candidates.into_iter().take(MAX_LOGO_CANDIDATES) {
        let url = format!("https://raw.githubusercontent.com/{owner}/{name}/{branch}/{path}");
        if is_square_icon(owner, name, branch, &path, &url) {
            return Ok(Some(url));
        }
    }

    Ok(Some(format!("https://github.com/{owner}.png?size=200")))
}

fn fetch_tree(owner: &str, name: &str, branch: &str) -> Option<Vec<GithubTreeEntry>> {
    let response = http_client()
        .ok()?
        .get(format!(
            "https://api.github.com/repos/{owner}/{name}/git/trees/{branch}?recursive=1"
        ))
        .send()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let tree = response.json::<GithubTree>().ok()?;
    Some(tree.tree)
}

/// Icons a project declares in its own build manifest. Those are the icon the
/// application actually ships with, so they beat any name-based guess.
fn declared_icons(
    owner: &str,
    name: &str,
    branch: &str,
    tree: &[GithubTreeEntry],
) -> Vec<String> {
    const MANIFESTS: [&str; 6] = [
        "tauri.conf.json",
        "electron-builder.json",
        "electron-builder.yml",
        "forge.config.js",
        ".claude-plugin/plugin.json",
        ".codex-plugin/plugin.json",
    ];

    let mut icons = Vec::new();
    for manifest in MANIFESTS {
        let Some(entry) = tree
            .iter()
            .find(|entry| entry.kind == "blob" && entry.path.eq_ignore_ascii_case(manifest))
        else {
            continue;
        };
        let Ok(text) = read_repo_file(owner, name, branch, &entry.path) else {
            continue;
        };
        // Icons are declared as `"icon": "path"`, `icon: path` or
        // `iconPath: path`; a permissive scan avoids a YAML/JS parser.
        for line in text.lines() {
            if !line.contains("icon") {
                continue;
            }
            let Some(value) = line.split(':').nth(1) else {
                continue;
            };
            let value = value.trim().trim_matches(|ch| matches!(ch, '"' | '\'' | ','));
            if value.is_empty() || value.contains('*') || !is_image_path(value) {
                continue;
            }
            let base = entry.path.rsplit_once('/').map(|(dir, _)| dir);
            let path = match base {
                Some(dir) if !value.starts_with('/') => {
                    format!("{}/{}", dir.trim_end_matches('/'), value.trim_start_matches("./"))
                }
                _ => value.trim_start_matches("./").to_string(),
            };
            if !icons.contains(&path) {
                icons.push(path);
            }
        }
    }
    icons
}

fn read_repo_file(owner: &str, name: &str, branch: &str, path: &str) -> Result<String, String> {
    let response = http_client()?
        .get(format!(
            "https://raw.githubusercontent.com/{owner}/{name}/{branch}/{path}"
        ))
        .send()
        .map_err(|err| format!("Could not reach GitHub: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("Could not read {path} from GitHub."));
    }
    let text = response
        .text()
        .map_err(|err| format!("Could not read {path} from GitHub: {err}"))?;
    // Manifests are small; refuse anything that is clearly not a manifest.
    Ok(text.chars().take(64 * 1024).collect())
}

fn is_image_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".svg", ".png", ".webp", ".jpg", ".jpeg", ".ico"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// Verifies a candidate really is a square-ish icon instead of a wide README
/// banner or screenshot. Undecodable candidates are rejected as well.
fn is_square_icon(owner: &str, name: &str, branch: &str, path: &str, url: &str) -> bool {
    let dimensions = if path.to_ascii_lowercase().ends_with(".svg") {
        svg_dimensions(owner, name, branch, path)
    } else {
        raster_dimensions(url)
    };
    dimensions.is_some_and(|(width, height)| icon_shape_ok(width, height))
}

fn icon_shape_ok(width: f64, height: f64) -> bool {
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    let ratio = width / height;
    let square = (0.75..=1.34).contains(&ratio);
    // Very small rasters blur once the tile scales them up.
    square && (width >= f64::from(MIN_LOGO_EDGE) || height >= f64::from(MIN_LOGO_EDGE))
}

/// `width`/`height` or `viewBox` from the first few hundred bytes of an SVG.
/// The tail of the file is not fetched: attributes live in the root element.
fn svg_dimensions(owner: &str, name: &str, branch: &str, path: &str) -> Option<(f64, f64)> {
    parse_svg_dimensions(&read_repo_file(owner, name, branch, path).ok()?)
}

/// `width`/`height` or `viewBox` from the root element of an SVG document.
/// Percentages and other units are ignored, so `viewBox` wins there.
fn parse_svg_dimensions(text: &str) -> Option<(f64, f64)> {
    if !text.trim_start().starts_with('<') {
        return None;
    }
    let head: String = text.chars().take(4096).collect();
    let raw = |key: &str| {
        let start = head.find(&format!("{key}="))? + key.len() + 1;
        let rest = &head[start..];
        let quote = rest.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let end = rest[1..].find(quote)? + 1;
        Some(rest[1..end].to_string())
    };
    // `100%`, `12pt` and friends are not pixel sizes; only bare numbers are.
    let length = |key: &str| {
        raw(key)?
            .trim_end_matches("px")
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| *value > 0.0)
    };
    if let (Some(width), Some(height)) = (length("width"), length("height")) {
        return Some((width, height));
    }
    let view_box = raw("viewBox")?;
    let mut parts = view_box.split_whitespace().filter_map(|part| part.parse::<f64>().ok());
    let _x = parts.next()?;
    let _y = parts.next()?;
    Some((parts.next()?, parts.next()?))
}

/// Width and height from the PNG/JPEG/WebP/GIF header. Only the header bytes
/// are downloaded, so this stays cheap even for large artwork.
fn raster_dimensions(url: &str) -> Option<(f64, f64)> {
    let response = http_client()
        .ok()?
        .get(url)
        .header("Range", "bytes=0-65535")
        .send()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().ok()?;
    let bytes = bytes.as_ref();
    let dimensions = png_dimensions(bytes)
        .or_else(|| gif_dimensions(bytes))
        .or_else(|| webp_dimensions(bytes))
        .or_else(|| jpeg_dimensions(bytes))?;
    Some((f64::from(dimensions.0), f64::from(dimensions.1)))
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    // Signature (8) + length (4) + "IHDR" (4) + width (4) + height (4).
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

fn gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 || (&bytes[..4] != b"GIF8") {
        return None;
    }
    let width = u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32;
    let height = u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32;
    Some((width, height))
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 30 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    match &bytes[12..16] {
        b"VP8X" => {
            let width = 1 + u32::from(bytes[24]) + (u32::from(bytes[25]) << 8) + (u32::from(bytes[26]) << 16);
            let height = 1 + u32::from(bytes[27]) + (u32::from(bytes[28]) << 8) + (u32::from(bytes[29]) << 16);
            Some((width, height))
        }
        b"VP8 " => {
            let width = u16::from_le_bytes(bytes[26..28].try_into().ok()?) as u32 & 0x3fff;
            let height = u16::from_le_bytes(bytes[28..30].try_into().ok()?) as u32 & 0x3fff;
            Some((width, height))
        }
        b"VP8L" => {
            let bits = u32::from_le_bytes(bytes[21..25].try_into().ok()?);
            let width = (bits & 0x3fff) + 1;
            let height = ((bits >> 14) & 0x3fff) + 1;
            Some((width, height))
        }
        _ => None,
    }
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut index = 2;
    while index + 9 < bytes.len() {
        if bytes[index] != 0xff {
            index += 1;
            continue;
        }
        let marker = bytes[index + 1];
        let length = u16::from_be_bytes([bytes[index + 2], bytes[index + 3]]) as usize;
        // SOF0..SOF15 except the DHT/JPG/DAC markers carry the frame size.
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let height = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
            let width = u16::from_be_bytes([bytes[index + 7], bytes[index + 8]]) as u32;
            return Some((width, height));
        }
        index += 2 + length;
    }
    None
}

/// Lower scores win. `None` excludes the file. The score is
/// `name * 1000 + location * 100 + depth * 10 + extension`, so a well-named
/// icon in a conventional location always beats a stray `logo.png`.
fn logo_score(path: &str) -> Option<u32> {
    let lower = path.to_ascii_lowercase();
    // Assets that never represent the application itself: tooling metadata,
    // test fixtures, docs, screenshots, and wide marketing artwork.
    if [
        "readme/", "screenshot", "testdata", "tests/", "test/", "fixture", "example", "sample",
        "vendor/", "node_modules/", "docs/", "doc/", "donation", "qr", "button", "partner",
        "social", "banner", "preview", "coverage/", ".github/", ".vscode/", ".idea/",
        "wordmark", "logotype", "header", "hero", "cover", "og-image", "opengraph", "favicon-",
    ]
    .iter()
    .any(|part| lower.contains(part))
    {
        return None;
    }

    let file = lower.rsplit('/').next()?;
    let stem = file.split('.').next().unwrap_or(file);
    let extension = if lower.ends_with(".svg") {
        0
    } else if lower.ends_with(".png") {
        1
    } else if lower.ends_with(".webp") {
        2
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        3
    } else if lower.ends_with(".ico") {
        4
    } else {
        return None;
    };

    let name = if matches!(
        stem,
        "app-icon" | "appicon" | "app-logo" | "applogo" | "logo" | "icon" | "icon-512" | "icon-1024"
    ) {
        0
    } else if stem.contains("app-icon") || stem.contains("applogo") {
        1
    } else if stem.contains("logo") {
        2
    } else if stem.contains("icon") || stem.contains("mark") {
        3
    } else {
        return None;
    };

    // Framework conventions that hold the icon the app actually ships.
    let location = if lower.starts_with("src-tauri/icons/")
        || lower.contains("/icons/")
        || lower.contains("/resources/")
        || lower.starts_with("resources/")
        || lower.contains("/brand/")
        || lower.starts_with("brand/")
        || lower.starts_with("public/")
        || lower.contains("/public/")
        || lower.starts_with("assets/")
        || lower.contains("/assets/")
        || lower.starts_with("static/")
    {
        0
    } else if lower.starts_with(".codex-plugin/") || lower.starts_with(".claude-plugin/") {
        2
    } else if lower.starts_with("dist/") || lower.starts_with("build/") || lower.contains("/build/")
    {
        2
    } else {
        1
    };

    let depth = path.matches('/').count() as u32;
    Some(name * 1000 + location * 100 + depth * 10 + extension)
}

fn fetch_repo(owner: &str, name: &str) -> Result<GithubRepo, String> {
    let response = http_client()?
        .get(format!("https://api.github.com/repos/{owner}/{name}"))
        .send()
        .map_err(|err| format!("Could not reach GitHub: {err}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            404 => "GitHub repository not found (or it is private).".to_string(),
            403 => "GitHub API rate limit exceeded. Try again later.".to_string(),
            _ => format!("GitHub API returned {status}."),
        });
    }

    response
        .json::<GithubRepo>()
        .map_err(|err| format!("Invalid response from GitHub: {err}"))
}

/// Downloads the root README through GitHub's readme endpoint, which resolves
/// the file name's casing and extension (`README.md`, `readme.rst`, ...).
fn fetch_readme(owner: &str, name: &str) -> Result<Vec<u8>, String> {
    let response = http_client()?
        .get(format!(
            "https://api.github.com/repos/{owner}/{name}/readme"
        ))
        .header("Accept", "application/vnd.github.raw")
        .send()
        .map_err(|err| format!("Could not reach GitHub: {err}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            404 => "This repository has no README at its root.".to_string(),
            403 => "GitHub API rate limit exceeded. Try again later.".to_string(),
            _ => format!("GitHub API returned {status}."),
        });
    }

    response
        .bytes()
        .map(|bytes| bytes.to_vec())
        .map_err(|err| format!("Failed to read README from GitHub: {err}"))
}

#[cfg(test)]
mod tests {
    use super::{
        gif_dimensions, icon_shape_ok, jpeg_dimensions, logo_score, parse_repo_url,
        parse_svg_dimensions, png_dimensions, webp_dimensions,
    };

    #[test]
    fn parses_supported_urls() {
        for url in [
            "https://github.com/owner/repo",
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo/",
            " https://github.com/owner/repo.git/ ",
        ] {
            assert_eq!(
                parse_repo_url(url).unwrap(),
                ("owner".to_string(), "repo".to_string())
            );
        }
    }

    #[test]
    fn logo_score_prefers_app_icons_and_ignores_readme_assets() {
        let app_icon = logo_score("assets/app-icon.svg").unwrap();
        assert!(app_icon < logo_score("assets/app-icon.png").unwrap());
        assert!(app_icon < logo_score("web/public/beef-logo.png").unwrap());
        assert!(logo_score("assets/readme/beeftv-2.0.1-skills.webp").is_none());
        assert!(logo_score("docs/public/images/canvas-version-history.png").is_none());
        assert!(logo_score("README.md").is_none());
    }

    #[test]
    fn logo_score_rejects_tooling_and_marketing_artwork() {
        // Plugin resources belong to a tool, not to the application itself.
        let plugin_logo = logo_score(".codex-plugin/logo.png").unwrap();
        let framework_icon = logo_score("src-tauri/icons/icon.png").unwrap();
        assert!(framework_icon < plugin_logo);

        // Wide artwork and fixtures never make a usable square tile.
        assert!(logo_score("assets/wordmark.svg").is_none());
        assert!(logo_score("public/og-image.png").is_none());
        assert!(logo_score("tests/fixtures/icon.png").is_none());
        assert!(logo_score("examples/demo/logo.png").is_none());
        assert!(logo_score("node_modules/pkg/icon.png").is_none());
    }

    #[test]
    fn logo_score_prefers_framework_icon_directories() {
        assert!(
            logo_score("src-tauri/icons/icon.png").unwrap()
                < logo_score("misc/icon.png").unwrap()
        );
        assert!(logo_score("assets/app-icon.png").unwrap() < logo_score("logo.png").unwrap());
    }

    #[test]
    fn icon_shape_rejects_banners_and_tiny_rasters() {
        assert!(icon_shape_ok(512.0, 512.0));
        assert!(icon_shape_ok(256.0, 320.0));
        assert!(!icon_shape_ok(1200.0, 630.0));
        assert!(!icon_shape_ok(64.0, 64.0));
        assert!(!icon_shape_ok(0.0, 0.0));
    }

    #[test]
    fn parses_svg_dimensions_from_root_attributes() {
        let square = r#"<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256">"#;
        assert_eq!(parse_svg_dimensions(square), Some((256.0, 256.0)));

        let pixels = r#"<svg width='64px' height='64px'></svg>"#;
        assert_eq!(parse_svg_dimensions(pixels), Some((64.0, 64.0)));

        // Percentages are not pixel sizes, so the viewBox decides.
        let percent = r#"<svg width="100%" height="100%" viewBox="0 0 48 48"></svg>"#;
        assert_eq!(parse_svg_dimensions(percent), Some((48.0, 48.0)));

        // A wide banner reports a wide ratio and is rejected downstream.
        let banner = r#"<svg viewBox="0 0 1200 300"></svg>"#;
        let (width, height) = parse_svg_dimensions(banner).unwrap();
        assert!(!icon_shape_ok(width, height));

        assert_eq!(parse_svg_dimensions("<not-an-svg"), None);
        assert_eq!(parse_svg_dimensions(r#"<svg></svg>"#), None);
    }

    #[test]
    fn reads_raster_dimensions_from_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&512u32.to_be_bytes());
        png.extend_from_slice(&256u32.to_be_bytes());
        assert_eq!(png_dimensions(&png), Some((512, 256)));

        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&120u16.to_le_bytes());
        gif.extend_from_slice(&90u16.to_le_bytes());
        assert_eq!(gif_dimensions(&gif), Some((120, 90)));

        let mut webp = b"RIFF\x00\x00\x00\x00WEBPVP8 ".to_vec();
        webp.extend_from_slice(&[0; 10]);
        webp.extend_from_slice(&400u16.to_le_bytes());
        webp.extend_from_slice(&300u16.to_le_bytes());
        assert_eq!(webp_dimensions(&webp), Some((400, 300)));

        let mut jpeg = b"\xff\xd8".to_vec();
        jpeg.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
        jpeg.extend_from_slice(&720u16.to_be_bytes());
        jpeg.extend_from_slice(&1280u16.to_be_bytes());
        jpeg.extend_from_slice(&[0; 12]);
        assert_eq!(jpeg_dimensions(&jpeg), Some((1280, 720)));

        assert_eq!(png_dimensions(b"not an image"), None);
        assert_eq!(jpeg_dimensions(b"not an image"), None);
    }

    #[test]
    fn rejects_invalid_urls() {
        for url in [
            "",
            "owner/repo",
            "http://github.com/owner/repo",
            "https://gitlab.com/owner/repo",
            "https://github.com/owner",
            "https://github.com/owner/repo/tree/main",
            "https://github.com/owner/repo?tab=readme",
            "https://github.com/../repo",
            "https://github.com/owner/repo;rm",
        ] {
            assert!(parse_repo_url(url).is_err(), "expected {url:?} to fail");
        }
    }
}