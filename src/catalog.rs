use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::probe;

const BUNDLED_RECIPES: &[&str] = &[include_str!("../catalog/ollama.json")];
const IMPORT_SUBDIR: &str = "catalog/imported";
const MAX_RECIPE_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub glyph: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub platforms: Vec<String>,
    pub install: Option<Install>,
    pub service: Option<Service>,
    pub uninstall: Option<Uninstall>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Install {
    pub kind: String,
    pub repo: String,
    #[serde(default)]
    pub asset: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Service {
    pub command: Vec<String>,
    pub health_url: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Uninstall {
    #[serde(default)]
    pub keep_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSource {
    pub url: String,
    pub sha256: String,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredRecipe {
    recipe: Recipe,
    source: RecipeSource,
}

#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub recipe: Recipe,
    pub source: Option<RecipeSource>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipePreview {
    pub recipe: Recipe,
    pub source_url: String,
    pub sha256: String,
    pub platform: String,
    pub supported: bool,
    pub conflict: bool,
    pub warnings: Vec<String>,
}

impl Recipe {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("Recipe is missing an id.".to_string());
        }
        if !self
            .id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        {
            return Err(format!("Recipe id '{}' contains unsupported characters.", self.id));
        }
        if self.name.trim().is_empty() {
            return Err(format!("Recipe '{}' is missing a name.", self.id));
        }
        if self.platforms.is_empty() {
            return Err(format!(
                "Recipe '{}' does not declare any supported platforms.",
                self.id
            ));
        }

        let install = self.install.as_ref().ok_or_else(|| {
            format!("Recipe '{}' is missing an install section.", self.id)
        })?;
        if install.kind != "github-release-binary" {
            return Err(format!(
                "Recipe '{}' uses unsupported install kind '{}'.",
                self.id, install.kind
            ));
        }
        if !install.repo.contains('/') {
            return Err(format!(
                "Recipe '{}' has an invalid GitHub repository '{}'.",
                self.id, install.repo
            ));
        }
        if install.asset.is_empty() {
            return Err(format!(
                "Recipe '{}' does not declare a release asset.",
                self.id
            ));
        }

        let service = self.service.as_ref().ok_or_else(|| {
            format!("Recipe '{}' is missing a service section.", self.id)
        })?;
        if service.command.is_empty() || service.command[0].trim().is_empty() {
            return Err(format!("Recipe '{}' is missing a start command.", self.id));
        }
        if !(service.health_url.starts_with("http://")
            || service.health_url.starts_with("https://"))
        {
            return Err(format!(
                "Recipe '{}' has an invalid health URL '{}'.",
                self.id, service.health_url
            ));
        }

        if self.uninstall.is_none() {
            return Err(format!(
                "Recipe '{}' is missing an uninstall section.",
                self.id
            ));
        }

        Ok(())
    }

    pub fn validate_for_import(&self, platform: &str) -> Result<(), String> {
        self.validate()?;
        let service = self
            .service
            .as_ref()
            .ok_or_else(|| format!("Recipe '{}' is missing a service section.", self.id))?;
        for token in &service.command {
            let normalized = token.trim().to_ascii_lowercase();
            if matches!(normalized.as_str(), "sudo" | "pkexec" | "doas") {
                return Err(format!(
                    "Recipe '{}' requires elevation ('{}'), which is not supported.",
                    self.id, token
                ));
            }
        }
        if !self.platforms.iter().any(|supported| supported == platform) {
            return Err(format!(
                "Recipe '{}' does not support this machine ({platform}).",
                self.id
            ));
        }
        Ok(())
    }
}

pub fn bundled() -> Result<Vec<Recipe>, String> {
    static CACHE: OnceLock<Result<Vec<Recipe>, String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let mut recipes = Vec::new();
            for raw in BUNDLED_RECIPES {
                let recipe: Recipe = serde_json::from_str(raw)
                    .map_err(|err| format!("Invalid bundled recipe: {err}"))?;
                recipe.validate()?;
                recipes.push(recipe);
            }
            Ok(recipes)
        })
        .clone()
}

pub fn entries(app_data: &Path) -> Result<Vec<CatalogEntry>, String> {
    let mut entries: Vec<CatalogEntry> = bundled()?
        .into_iter()
        .map(|recipe| CatalogEntry {
            recipe,
            source: None,
        })
        .collect();

    let directory = imported_dir(app_data);
    if directory.is_dir() {
        let mut imported = Vec::new();
        for item in fs::read_dir(&directory)
            .map_err(|err| format!("Failed to read the imported catalog: {err}"))?
        {
            let path = item
                .map_err(|err| format!("Failed to read an imported recipe: {err}"))?
                .path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(stored) = serde_json::from_str::<StoredRecipe>(&raw) else {
                continue;
            };
            if stored.recipe.validate().is_err() {
                continue;
            }
            if entries
                .iter()
                .any(|entry| entry.recipe.id == stored.recipe.id)
            {
                continue;
            }
            imported.push(CatalogEntry {
                recipe: stored.recipe,
                source: Some(stored.source),
            });
        }
        imported.sort_by(|left, right| left.recipe.id.cmp(&right.recipe.id));
        entries.extend(imported);
    }

    Ok(entries)
}

pub fn find(app_data: &Path, app_id: &str) -> Result<CatalogEntry, String> {
    entries(app_data)?
        .into_iter()
        .find(|entry| entry.recipe.id == app_id)
        .ok_or_else(|| format!("Unknown application '{app_id}'."))
}

pub fn preview_recipe(app_data: &Path, input: &str) -> Result<RecipePreview, String> {
    let (source_url, content) = fetch_recipe_source(input)?;
    let sha256 = sha256_hex(content.as_bytes());
    let recipe = parse_recipe(&content)?;
    let platform = probe::current_platform_id()?;
    let supported = recipe.platforms.iter().any(|item| item == &platform);
    let conflict = entries(app_data)?
        .iter()
        .any(|entry| entry.recipe.id == recipe.id);

    let mut warnings = vec![
        "Release archives are not checksum-verified by this version.".to_string(),
    ];
    if !supported {
        warnings.push(format!(
            "This recipe does not support the current machine ({platform})."
        ));
    }
    if conflict {
        warnings.push(format!(
            "An application with id '{}' already exists.",
            recipe.id
        ));
    }

    Ok(RecipePreview {
        recipe,
        source_url,
        sha256,
        platform,
        supported,
        conflict,
        warnings,
    })
}

pub fn import_recipe(
    app_data: &Path,
    input: &str,
    expected_sha256: &str,
) -> Result<CatalogEntry, String> {
    if expected_sha256.trim().is_empty() {
        return Err("The recipe preview is missing its checksum. Preview it again.".to_string());
    }
    let (source_url, content) = fetch_recipe_source(input)?;
    let sha256 = sha256_hex(content.as_bytes());
    if sha256 != expected_sha256 {
        return Err("The recipe changed since it was previewed. Preview it again.".to_string());
    }
    let recipe = parse_recipe(&content)?;
    let platform = probe::current_platform_id()?;
    recipe.validate_for_import(&platform)?;

    if entries(app_data)?
        .iter()
        .any(|entry| entry.recipe.id == recipe.id)
    {
        return Err(format!(
            "An application with id '{}' already exists.",
            recipe.id
        ));
    }

    let imported_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string());
    let source = RecipeSource {
        url: source_url,
        sha256,
        imported_at,
    };
    store_imported(app_data, &recipe, &source)?;
    Ok(CatalogEntry {
        recipe,
        source: Some(source),
    })
}

pub fn remove_recipe(app_data: &Path, app_id: &str) -> Result<(), String> {
    let path = imported_path(app_data, app_id);
    if !path.exists() {
        return Err(format!(
            "'{app_id}' is not an imported application and cannot be removed."
        ));
    }
    fs::remove_file(&path)
        .map_err(|err| format!("Failed to remove the imported recipe: {err}"))
}

fn store_imported(
    app_data: &Path,
    recipe: &Recipe,
    source: &RecipeSource,
) -> Result<(), String> {
    let directory = imported_dir(app_data);
    fs::create_dir_all(&directory)
        .map_err(|err| format!("Failed to create the catalog directory: {err}"))?;
    let stored = StoredRecipe {
        recipe: recipe.clone(),
        source: source.clone(),
    };
    let content = serde_json::to_vec_pretty(&stored)
        .map_err(|err| format!("Failed to serialize the recipe: {err}"))?;
    let path = imported_path(app_data, &recipe.id);
    let temporary = directory.join(format!(".{}.tmp", recipe.id));
    fs::write(&temporary, content)
        .map_err(|err| format!("Failed to write the imported recipe: {err}"))?;
    fs::rename(&temporary, &path)
        .map_err(|err| format!("Failed to activate the imported recipe: {err}"))
}

fn imported_dir(app_data: &Path) -> PathBuf {
    app_data.join(IMPORT_SUBDIR)
}

fn imported_path(app_data: &Path, app_id: &str) -> PathBuf {
    imported_dir(app_data).join(format!("{app_id}.json"))
}

fn parse_recipe(content: &str) -> Result<Recipe, String> {
    let recipe: Recipe = serde_json::from_str(content)
        .map_err(|err| format!("The URL does not contain a valid EasyAI recipe: {err}"))?;
    recipe.validate()?;
    Ok(recipe)
}

fn fetch_recipe_source(input: &str) -> Result<(String, String), String> {
    let url = reqwest::Url::parse(input.trim())
        .map_err(|_| "Enter a valid recipe or GitHub repository URL.".to_string())?;
    ensure_allowed_scheme(&url)?;

    if let Some((owner, repo)) = github_repo_parts(&url) {
        let candidates = [
            format!("https://raw.githubusercontent.com/{owner}/{repo}/HEAD/easyai.json"),
            format!("https://raw.githubusercontent.com/{owner}/{repo}/HEAD/.easyai/recipe.json"),
        ];
        let mut last_error = String::new();
        for candidate in candidates {
            match http_get(&candidate) {
                Ok(content) => return Ok((candidate, content)),
                Err(err) => last_error = err,
            }
        }
        return Err(format!(
            "This repository does not publish an EasyAI recipe (looked for easyai.json and .easyai/recipe.json). {last_error}"
        ));
    }

    let content = http_get(url.as_str())?;
    Ok((url.to_string(), content))
}

fn ensure_allowed_scheme(url: &reqwest::Url) -> Result<(), String> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback(url) => Ok(()),
        _ => Err(
            "Recipe URLs must use https:// (http:// is allowed only for localhost).".to_string(),
        ),
    }
}

fn is_loopback(url: &reqwest::Url) -> bool {
    matches!(
        url.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("::1")
    )
}

fn github_repo_parts(url: &reqwest::Url) -> Option<(String, String)> {
    if url.scheme() != "https" || url.host_str() != Some("github.com") {
        return None;
    }
    let segments: Vec<&str> = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() != 2 {
        return None;
    }
    let owner = segments[0];
    let repo = segments[1].strip_suffix(".git").unwrap_or(segments[1]);
    if !is_valid_github_name(owner) || !is_valid_github_name(repo) {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

fn is_valid_github_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn http_get(url: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("EasyAI/0.1")
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|err| format!("Failed to create HTTP client: {err}"))?;
    let response = client
        .get(url)
        .send()
        .map_err(|err| format!("Could not fetch the recipe: {err}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            404 => "Recipe not found (404).".to_string(),
            403 => "The recipe server denied access (403).".to_string(),
            _ => format!("The recipe server returned {status}."),
        });
    }
    if let Some(length) = response.content_length() {
        if length > MAX_RECIPE_BYTES {
            return Err("The recipe is too large.".to_string());
        }
    }
    let mut buffer = Vec::new();
    response
        .take(MAX_RECIPE_BYTES + 1)
        .read_to_end(&mut buffer)
        .map_err(|err| format!("Failed to read the recipe: {err}"))?;
    if buffer.len() as u64 > MAX_RECIPE_BYTES {
        return Err("The recipe is too large.".to_string());
    }
    String::from_utf8(buffer).map_err(|_| "The recipe is not valid UTF-8.".to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use serde_json::{json, Value};

    use super::{
        entries, find, github_repo_parts, import_recipe, preview_recipe, remove_recipe,
        sha256_hex, store_imported, Recipe, RecipeSource,
    };

    fn base_recipe() -> Value {
        json!({
            "id": "demo",
            "name": "Demo",
            "platforms": ["linux-x86_64"],
            "install": {
                "kind": "github-release-binary",
                "repo": "owner/repo",
                "asset": { "linux-x86_64": "demo.tgz" }
            },
            "service": {
                "command": ["{install_dir}/bin/demo", "serve"],
                "health_url": "http://127.0.0.1:9999/health"
            },
            "uninstall": { "keep_paths": [] }
        })
    }

    fn parse(value: Value) -> Recipe {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn bundled_recipe_loads_and_validates() {
        let recipes = super::bundled().unwrap();
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0].id, "ollama");
        assert!(recipes[0].platforms.contains(&"linux-x86_64".to_string()));
    }

    #[test]
    fn valid_recipe_passes() {
        assert!(parse(base_recipe()).validate().is_ok());
    }

    #[test]
    fn missing_id_is_rejected() {
        let mut value = base_recipe();
        value["id"] = json!("");
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("id"), "{err}");
    }

    #[test]
    fn missing_platforms_is_rejected() {
        let mut value = base_recipe();
        value["platforms"] = json!([]);
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("platform"), "{err}");
    }

    #[test]
    fn missing_install_is_rejected() {
        let mut value = base_recipe();
        value.as_object_mut().unwrap().remove("install");
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("install"), "{err}");
    }

    #[test]
    fn unsupported_install_kind_is_rejected() {
        let mut value = base_recipe();
        value["install"]["kind"] = json!("pip");
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("install kind"), "{err}");
    }

    #[test]
    fn missing_service_is_rejected() {
        let mut value = base_recipe();
        value.as_object_mut().unwrap().remove("service");
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("service"), "{err}");
    }

    #[test]
    fn empty_command_is_rejected() {
        let mut value = base_recipe();
        value["service"]["command"] = json!([]);
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("start command"), "{err}");
    }

    #[test]
    fn invalid_health_url_is_rejected() {
        let mut value = base_recipe();
        value["service"]["health_url"] = json!("127.0.0.1:9999");
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("health URL"), "{err}");
    }

    #[test]
    fn missing_uninstall_is_rejected() {
        let mut value = base_recipe();
        value.as_object_mut().unwrap().remove("uninstall");
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("uninstall"), "{err}");
    }

    #[test]
    fn import_rejects_unsupported_platform() {
        let recipe = parse(base_recipe());
        let err = recipe.validate_for_import("macos-aarch64").unwrap_err();
        assert!(err.contains("does not support"), "{err}");
        assert!(recipe.validate_for_import("linux-x86_64").is_ok());
    }

    #[test]
    fn import_rejects_elevation_commands() {
        let mut value = base_recipe();
        value["service"]["command"] = json!(["sudo", "{install_dir}/bin/demo"]);
        let recipe = parse(value);
        let err = recipe
            .validate_for_import("linux-x86_64")
            .unwrap_err();
        assert!(err.contains("elevation"), "{err}");
    }

    #[test]
    fn sha256_matches_known_value() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn identifies_github_repository_urls() {
        let url = reqwest::Url::parse("https://github.com/owner/repo").unwrap();
        assert_eq!(
            github_repo_parts(&url),
            Some(("owner".to_string(), "repo".to_string()))
        );
        let url = reqwest::Url::parse("https://github.com/owner/repo.git").unwrap();
        assert_eq!(
            github_repo_parts(&url),
            Some(("owner".to_string(), "repo".to_string()))
        );
        let url = reqwest::Url::parse("https://github.com/owner/repo/tree/main").unwrap();
        assert_eq!(github_repo_parts(&url), None);
        let url = reqwest::Url::parse("https://raw.githubusercontent.com/owner/repo/x").unwrap();
        assert_eq!(github_repo_parts(&url), None);
    }

    fn serve_recipe(body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let mut stream = stream.unwrap();
                let mut buffer = [0u8; 2048];
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{address}/easyai.json")
    }

    fn remote_recipe(platform: &str) -> Value {
        json!({
            "id": "remote-demo",
            "name": "Remote demo",
            "platforms": [platform],
            "install": {
                "kind": "github-release-binary",
                "repo": "owner/repo",
                "asset": { platform: "demo.tgz" }
            },
            "service": {
                "command": ["{install_dir}/bin/demo", "serve"],
                "health_url": "http://127.0.0.1:9999/health"
            },
            "uninstall": { "keep_paths": [] }
        })
    }

    fn temp_catalog_dir(label: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "easyai-catalog-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn previews_and_imports_from_a_recipe_url() {
        let platform = crate::probe::current_platform_id().unwrap();
        let body = remote_recipe(&platform).to_string();
        let url = serve_recipe(body);
        let directory = temp_catalog_dir("import-url");

        let preview = preview_recipe(&directory, &url).unwrap();
        assert!(preview.supported);
        assert!(!preview.conflict);
        assert_eq!(preview.recipe.id, "remote-demo");

        let entry = import_recipe(&directory, &url, &preview.sha256).unwrap();
        assert_eq!(entry.recipe.id, "remote-demo");
        let found = find(&directory, "remote-demo").unwrap();
        assert_eq!(found.source.as_ref().unwrap().url, url);

        remove_recipe(&directory, "remote-demo").unwrap();
        assert!(find(&directory, "remote-demo").is_err());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn import_rejects_a_changed_recipe() {
        let platform = crate::probe::current_platform_id().unwrap();
        let url = serve_recipe(remote_recipe(&platform).to_string());
        let directory = temp_catalog_dir("import-changed");

        let err = import_recipe(&directory, &url, "deadbeef").unwrap_err();
        assert!(err.contains("changed"), "{err}");
        assert!(find(&directory, "remote-demo").is_err());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn preview_reports_bundled_conflict() {
        let platform = crate::probe::current_platform_id().unwrap();
        let mut value = remote_recipe(&platform);
        value["id"] = json!("ollama");
        let url = serve_recipe(value.to_string());
        let directory = temp_catalog_dir("import-conflict");

        let preview = preview_recipe(&directory, &url).unwrap();
        assert!(preview.conflict);
        let err = import_recipe(&directory, &url, &preview.sha256).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn imported_recipe_roundtrip() {
        let directory = std::env::temp_dir().join(format!(
            "easyai-catalog-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();

        let recipe = parse(base_recipe());
        let source = RecipeSource {
            url: "https://example.com/easyai.json".to_string(),
            sha256: "abc".to_string(),
            imported_at: "0".to_string(),
        };
        store_imported(&directory, &recipe, &source).unwrap();

        let all = entries(&directory).unwrap();
        assert_eq!(all.len(), 2);
        let imported = find(&directory, "demo").unwrap();
        assert_eq!(
            imported.source.as_ref().unwrap().url,
            "https://example.com/easyai.json"
        );

        remove_recipe(&directory, "demo").unwrap();
        assert!(find(&directory, "demo").is_err());
        assert_eq!(entries(&directory).unwrap().len(), 1);

        std::fs::remove_dir_all(&directory).unwrap();
    }
}

