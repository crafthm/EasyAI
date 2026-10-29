use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    #[serde(default)]
    pub methods: Vec<Method>,
    #[serde(default, rename = "install", skip_serializing_if = "Option::is_none")]
    pub legacy_install: Option<serde_json::Value>,
    #[serde(default, rename = "service", skip_serializing_if = "Option::is_none")]
    pub legacy_service: Option<serde_json::Value>,
    #[serde(default, rename = "uninstall", skip_serializing_if = "Option::is_none")]
    pub legacy_uninstall: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MethodKind {
    Release,
    Portable,
    Container,
    PackageManager,
    Language,
    Script,
    Source,
    Vcs,
    Universal,
}

impl MethodKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MethodKind::Release => "release",
            MethodKind::Portable => "portable",
            MethodKind::Container => "container",
            MethodKind::PackageManager => "package-manager",
            MethodKind::Language => "language",
            MethodKind::Script => "script",
            MethodKind::Source => "source",
            MethodKind::Vcs => "vcs",
            MethodKind::Universal => "universal",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Method {
    pub id: String,
    pub kind: MethodKind,
    #[serde(default)]
    pub when: MethodWhen,
    #[serde(default)]
    pub prefer: i32,
    #[serde(default)]
    pub advanced: bool,
    #[serde(default)]
    pub elevation: bool,
    pub version: Option<VersionSource>,
    pub service: Option<Service>,
    pub uninstall: Option<Uninstall>,
    pub size: Option<u64>,
    pub source: Option<MethodSource>,
    pub image: Option<ImageSpec>,
    pub package: Option<PackageSpec>,
    pub language: Option<LanguageSpec>,
    pub script: Option<ScriptSpec>,
    pub universal: Option<UniversalSpec>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MethodWhen {
    #[serde(default)]
    pub os: Vec<String>,
    #[serde(default)]
    pub arch: Vec<String>,
    #[serde(default)]
    pub gpu: Vec<String>,
    #[serde(default)]
    pub runtime: Vec<String>,
    #[serde(default)]
    pub manager: Vec<String>,
    #[serde(default)]
    pub toolchain: Vec<String>,
    pub min_ram_mb: Option<u64>,
    pub min_disk_mb: Option<u64>,
}

impl MethodWhen {
    pub fn matches_platform(&self, os: &str, arch: &str) -> bool {
        (self.os.is_empty() || self.os.iter().any(|item| item == os))
            && (self.arch.is_empty() || self.arch.iter().any(|item| item == arch))
    }

    pub fn matches_gpu(&self, gpus: &[String]) -> bool {
        self.gpu.is_empty()
            || self.gpu.iter().any(|required| {
                gpus.iter().any(|available| available == required)
            })
    }

    pub fn matches_runtimes(&self, runtimes: &HashSet<String>) -> bool {
        self.runtime.is_empty()
            || self.runtime.iter().any(|required| runtimes.contains(required))
    }

    pub fn matches_managers(&self, managers: &HashSet<String>) -> bool {
        self.manager.is_empty()
            || self.manager.iter().any(|required| managers.contains(required))
    }

    pub fn matches_toolchains(&self, toolchains: &HashSet<String>) -> bool {
        self.toolchain.is_empty()
            || self
                .toolchain
                .iter()
                .any(|required| toolchains.contains(required))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionSource {
    #[serde(rename = "type")]
    pub kind: String,
    pub repo: Option<String>,
    pub command: Option<Vec<String>>,
    pub package: Option<String>,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Service {
    #[serde(default)]
    pub command: Vec<String>,
    pub health_url: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Uninstall {
    #[serde(default)]
    pub keep_paths: Vec<String>,
    #[serde(default)]
    pub remove_paths: Vec<String>,
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub native: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MethodSource {
    pub repo: Option<String>,
    #[serde(default)]
    pub asset: HashMap<String, String>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub git: Option<String>,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub build: Option<BuildSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BuildSpec {
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub commands: Vec<Vec<String>>,
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageSpec {
    pub image: String,
    pub tag: Option<String>,
    pub digest: Option<String>,
    pub runtime: Option<String>,
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(default)]
    pub ports: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub gpu: Option<String>,
    pub restart: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackageSpec {
    pub manager: String,
    pub package: String,
    pub tap: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LanguageSpec {
    pub ecosystem: String,
    pub package: String,
    pub version: Option<String>,
    pub index: Option<String>,
    pub extra_index: Option<String>,
    pub manager: Option<String>,
    pub executable: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScriptSpec {
    pub url: String,
    pub sha256: Option<String>,
    pub probe: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UniversalSpec {
    pub format: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub command: Option<Vec<String>>,
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
        if self.legacy_install.is_some() {
            return Err(format!(
                "Recipe '{}' uses the old 'install' field; migrate it to 'methods'.",
                self.id
            ));
        }
        if self.legacy_service.is_some() || self.legacy_uninstall.is_some() {
            return Err(format!(
                "Recipe '{}' uses old top-level service/uninstall fields; move them into each method.",
                self.id
            ));
        }
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
        if self.methods.is_empty() {
            return Err(format!("Recipe '{}' declares no installation methods.", self.id));
        }

        let mut ids = HashSet::new();
        for method in &self.methods {
            method.validate(&self.id)?;
            if !ids.insert(method.id.clone()) {
                return Err(format!(
                    "Recipe '{}' declares duplicate method id '{}'.",
                    self.id, method.id
                ));
            }
        }
        Ok(())
    }

    pub fn method(&self, method_id: &str) -> Option<&Method> {
        self.methods.iter().find(|method| method.id == method_id)
    }

    pub fn default_method(&self) -> Option<&Method> {
        self.methods
            .iter()
            .max_by_key(|method| method.prefer)
    }

    pub fn supports_platform(&self, os: &str, arch: &str) -> bool {
        self.methods
            .iter()
            .any(|method| method.when.matches_platform(os, arch))
    }

    pub fn validate_for_import(&self, platform: &str) -> Result<(), String> {
        self.validate()?;
        let (os, arch) = split_platform(platform)?;
        if !self.supports_platform(os, arch) {
            return Err(format!(
                "Recipe '{}' does not support this machine ({platform}).",
                self.id
            ));
        }
        for method in &self.methods {
            for command in method.all_commands() {
                for token in command {
                    let normalized = token.trim().to_ascii_lowercase();
                    if matches!(normalized.as_str(), "sudo" | "pkexec" | "doas") {
                        return Err(format!(
                            "Recipe '{}' method '{}' requires elevation ('{}'), which is not supported for imported recipes.",
                            self.id, method.id, token
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl Method {
    pub fn validate(&self, recipe_id: &str) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err(format!(
                "Recipe '{}' has a method without an id.",
                recipe_id
            ));
        }
        if !self
            .id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        {
            return Err(format!(
                "Recipe '{}' method id '{}' contains unsupported characters.",
                recipe_id, self.id
            ));
        }
        if !(0..=100).contains(&self.prefer) {
            return Err(format!(
                "Recipe '{}' method '{}' has prefer outside 0..=100.",
                recipe_id, self.id
            ));
        }
        let version = self.version.as_ref().ok_or_else(|| {
            format!(
                "Recipe '{}' method '{}' is missing a version source.",
                recipe_id, self.id
            )
        })?;
        if version.kind.trim().is_empty() {
            return Err(format!(
                "Recipe '{}' method '{}' has an empty version source type.",
                recipe_id, self.id
            ));
        }
        if let Some(service) = &self.service {
            if service.health_url.is_empty()
                || !(service.health_url.starts_with("http://")
                    || service.health_url.starts_with("https://"))
            {
                return Err(format!(
                    "Recipe '{}' method '{}' has an invalid health URL.",
                    recipe_id, self.id
                ));
            }
            if service.command.is_empty() && self.kind != MethodKind::Container {
                return Err(format!(
                    "Recipe '{}' method '{}' is missing a start command.",
                    recipe_id, self.id
                ));
            }
        }
        let uninstall = self.uninstall.as_ref().ok_or_else(|| {
            format!(
                "Recipe '{}' method '{}' is missing an uninstall section.",
                recipe_id, self.id
            )
        })?;
        let implicit_app_removal = matches!(
            self.kind,
            MethodKind::Release
                | MethodKind::Portable
                | MethodKind::Language
                | MethodKind::Script
                | MethodKind::Source
                | MethodKind::Vcs
        );
        if !uninstall.native
            && uninstall.remove_paths.is_empty()
            && uninstall.command.is_none()
            && !implicit_app_removal
        {
            return Err(format!(
                "Recipe '{}' method '{}' has no uninstall behavior.",
                recipe_id, self.id
            ));
        }

        match self.kind {
            MethodKind::Release => {
                let source = self.kind_source(recipe_id)?;
                if source.repo.as_deref().unwrap_or("").is_empty()
                    || source.asset.is_empty()
                {
                    return Err(format!(
                        "Recipe '{}' method '{}' needs a repo and asset map.",
                        recipe_id, self.id
                    ));
                }
            }
            MethodKind::Portable => {
                let source = self.kind_source(recipe_id)?;
                if source.url.is_none()
                    && (source.repo.is_none() || source.asset.is_empty())
                {
                    return Err(format!(
                        "Recipe '{}' method '{}' needs a portable URL or repo asset.",
                        recipe_id, self.id
                    ));
                }
            }
            MethodKind::Container => {
                let image = self.image.as_ref().ok_or_else(|| {
                    format!(
                        "Recipe '{}' method '{}' is missing an image section.",
                        recipe_id, self.id
                    )
                })?;
                if image.image.trim().is_empty()
                    || (image.tag.is_none() && image.digest.is_none())
                {
                    return Err(format!(
                        "Recipe '{}' method '{}' needs an image and a tag or digest.",
                        recipe_id, self.id
                    ));
                }
            }
            MethodKind::PackageManager => {
                let package = self.package.as_ref().ok_or_else(|| {
                    format!(
                        "Recipe '{}' method '{}' is missing a package section.",
                        recipe_id, self.id
                    )
                })?;
                if package.manager.is_empty() || package.package.is_empty() {
                    return Err(format!(
                        "Recipe '{}' method '{}' needs a manager and package name.",
                        recipe_id, self.id
                    ));
                }
            }
            MethodKind::Language => {
                let language = self.language.as_ref().ok_or_else(|| {
                    format!(
                        "Recipe '{}' method '{}' is missing a language section.",
                        recipe_id, self.id
                    )
                })?;
                if language.ecosystem.is_empty() || language.package.is_empty() {
                    return Err(format!(
                        "Recipe '{}' method '{}' needs an ecosystem and package name.",
                        recipe_id, self.id
                    ));
                }
            }
            MethodKind::Script => {
                let script = self.script.as_ref().ok_or_else(|| {
                    format!(
                        "Recipe '{}' method '{}' is missing a script section.",
                        recipe_id, self.id
                    )
                })?;
                if !script.url.starts_with("https://") {
                    return Err(format!(
                        "Recipe '{}' method '{}' needs an https script URL.",
                        recipe_id, self.id
                    ));
                }
            }
            MethodKind::Source => {
                let source = self.kind_source(recipe_id)?;
                let build = source.build.as_ref().ok_or_else(|| {
                    format!(
                        "Recipe '{}' method '{}' is missing a build section.",
                        recipe_id, self.id
                    )
                })?;
                if source.git.is_none()
                    || source.git_ref.is_none()
                    || build.commands.is_empty()
                {
                    return Err(format!(
                        "Recipe '{}' method '{}' needs git, a pinned ref, and build commands.",
                        recipe_id, self.id
                    ));
                }
            }
            MethodKind::Vcs => {
                let source = self.kind_source(recipe_id)?;
                if source.git.is_none() || source.git_ref.is_none() {
                    return Err(format!(
                        "Recipe '{}' method '{}' needs git and a pinned ref.",
                        recipe_id, self.id
                    ));
                }
                if self.language.is_none() && self.package.is_none() {
                    return Err(format!(
                        "Recipe '{}' method '{}' needs a language or package section.",
                        recipe_id, self.id
                    ));
                }
            }
            MethodKind::Universal => {
                let universal = self.universal.as_ref().ok_or_else(|| {
                    format!(
                        "Recipe '{}' method '{}' is missing a universal section.",
                        recipe_id, self.id
                    )
                })?;
                if universal.format.is_empty() || universal.reference.is_empty() {
                    return Err(format!(
                        "Recipe '{}' method '{}' needs a format and reference.",
                        recipe_id, self.id
                    ));
                }
            }
        }
        Ok(())
    }

    fn kind_source(&self, recipe_id: &str) -> Result<&MethodSource, String> {
        self.source.as_ref().ok_or_else(|| {
            format!(
                "Recipe '{}' method '{}' is missing a source section.",
                recipe_id, self.id
            )
        })
    }

    pub fn all_commands(&self) -> Vec<&Vec<String>> {
        let mut commands = Vec::new();
        if let Some(service) = &self.service {
            if !service.command.is_empty() {
                commands.push(&service.command);
            }
        }
        if let Some(uninstall) = &self.uninstall {
            if let Some(command) = &uninstall.command {
                commands.push(command);
            }
        }
        if let Some(source) = &self.source {
            if let Some(build) = &source.build {
                for command in &build.commands {
                    commands.push(command);
                }
            }
        }
        if let Some(script) = &self.script {
            if let Some(probe) = &script.probe {
                commands.push(probe);
            }
        }
        commands
    }
}

pub fn split_platform(platform: &str) -> Result<(&str, &str), String> {
    platform
        .split_once('-')
        .ok_or_else(|| format!("Unsupported platform '{platform}'."))
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
    let platform = crate::probe::current_platform_id()?;
    let (os, arch) = split_platform(&platform)?;
    let supported = recipe.supports_platform(os, arch);
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
    let platform = crate::probe::current_platform_id()?;
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
        sha256_hex, store_imported, MethodKind, Recipe, RecipeSource,
    };

    fn base_recipe() -> Value {
        json!({
            "id": "demo",
            "name": "Demo",
            "platforms": ["linux-x86_64"],
            "methods": [{
                "id": "release",
                "kind": "release",
                "when": { "os": ["linux"], "arch": ["x86_64"] },
                "prefer": 80,
                "source": {
                    "repo": "owner/repo",
                    "asset": { "linux-x86_64": "demo.tar.gz" }
                },
                "version": { "type": "github-release", "repo": "owner/repo" },
                "service": {
                    "command": ["{install_dir}/bin/demo", "serve"],
                    "health_url": "http://127.0.0.1:9999/health"
                },
                "uninstall": { "keep_paths": [] }
            }]
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
        assert!(recipes[0].methods.iter().any(|m| m.kind == MethodKind::Release));
        assert!(recipes[0].methods.iter().any(|m| m.kind == MethodKind::Container));
    }

    #[test]
    fn valid_recipe_passes() {
        let recipe = parse(base_recipe());
        assert!(recipe.validate().is_ok());
        assert_eq!(recipe.methods.len(), 1);
    }

    #[test]
    fn legacy_install_is_rejected_with_migration_hint() {
        let mut value = base_recipe();
        value.as_object_mut().unwrap().remove("methods");
        value["install"] = json!({ "kind": "github-release-binary" });
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("old 'install'"), "{err}");
        assert!(err.contains("demo"), "{err}");
    }

    #[test]
    fn missing_methods_is_rejected() {
        let mut value = base_recipe();
        value["methods"] = json!([]);
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("no installation methods"), "{err}");
    }

    #[test]
    fn duplicate_method_ids_are_rejected() {
        let mut value = base_recipe();
        let method = value["methods"][0].clone();
        value["methods"].as_array_mut().unwrap().push(method);
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn unsupported_method_kind_is_rejected_by_serde() {
        let mut value = base_recipe();
        value["methods"][0]["kind"] = json!("pip");
        assert!(serde_json::from_value::<Recipe>(value).is_err());
    }

    #[test]
    fn method_without_version_is_rejected() {
        let mut value = base_recipe();
        value["methods"][0]
            .as_object_mut()
            .unwrap()
            .remove("version");
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("version source"), "{err}");
    }

    #[test]
    fn method_without_uninstall_is_rejected() {
        let mut value = base_recipe();
        value["methods"][0]
            .as_object_mut()
            .unwrap()
            .remove("uninstall");
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("uninstall"), "{err}");
    }

    #[test]
    fn container_requires_image_and_tag_or_digest() {
        let mut value = base_recipe();
        value["methods"] = json!([{
            "id": "container",
            "kind": "container",
            "version": { "type": "image", "image": "demo" },
            "image": { "image": "demo/demo" },
            "uninstall": { "native": true }
        }]);
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("tag or digest"), "{err}");
    }

    #[test]
    fn source_requires_pinned_ref_and_build_commands() {
        let mut value = base_recipe();
        value["methods"] = json!([{
            "id": "source",
            "kind": "source",
            "advanced": true,
            "source": {
                "git": "https://github.com/owner/repo",
                "build": { "commands": [] }
            },
            "version": { "type": "git" },
            "uninstall": { "remove_paths": ["{app_dir}"] }
        }]);
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("pinned ref"), "{err}");
    }

    #[test]
    fn every_kind_rejects_missing_required_fields() {
        let cases = [
            (
                json!({
                    "id": "package",
                    "kind": "package-manager",
                    "version": { "type": "registry", "package": "demo" },
                    "uninstall": { "native": true }
                }),
                "package section",
            ),
            (
                json!({
                    "id": "language",
                    "kind": "language",
                    "version": { "type": "registry", "package": "demo" },
                    "uninstall": { "keep_paths": [] }
                }),
                "language section",
            ),
            (
                json!({
                    "id": "script",
                    "kind": "script",
                    "version": { "type": "command", "command": ["demo"] },
                    "uninstall": { "command": ["demo"] }
                }),
                "script section",
            ),
            (
                json!({
                    "id": "universal",
                    "kind": "universal",
                    "version": { "type": "command", "command": ["demo"] },
                    "uninstall": { "native": true }
                }),
                "universal section",
            ),
            (
                json!({
                    "id": "vcs",
                    "kind": "vcs",
                    "source": { "git": "https://example.com/demo", "ref": "v1.0.0" },
                    "version": { "type": "git" },
                    "uninstall": { "keep_paths": [] }
                }),
                "language or package",
            ),
            (
                json!({
                    "id": "portable",
                    "kind": "portable",
                    "version": { "type": "github-release", "repo": "owner/repo" },
                    "uninstall": { "keep_paths": [] }
                }),
                "source section",
            ),
        ];

        for (method, expected) in cases {
            let mut value = base_recipe();
            value["methods"] = json!([method]);
            let err = parse(value).validate().unwrap_err();
            assert!(err.contains(expected), "expected {expected}, got {err}");
        }
    }

    #[test]
    fn imported_old_schema_is_not_loaded() {
        let directory = temp_catalog_dir("old-import");
        let imported_dir = directory.join("catalog/imported");
        std::fs::create_dir_all(&imported_dir).unwrap();
        let old = json!({
            "recipe": {
                "id": "legacy",
                "name": "Legacy",
                "platforms": ["linux-x86_64"],
                "install": {
                    "kind": "github-release-binary",
                    "repo": "owner/repo",
                    "asset": { "linux-x86_64": "demo.tgz" }
                },
                "service": {
                    "command": ["demo"],
                    "health_url": "http://127.0.0.1:9999"
                },
                "uninstall": { "keep_paths": [] }
            },
            "source": {
                "url": "https://example.com/easyai.json",
                "sha256": "abc",
                "importedAt": "0"
            }
        });
        std::fs::write(
            imported_dir.join("legacy.json"),
            serde_json::to_vec(&old).unwrap(),
        )
        .unwrap();
        assert!(find(&directory, "legacy").is_err());
        assert_eq!(entries(&directory).unwrap().len(), 1);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn imported_recipe_rejects_elevation() {
        let mut value = base_recipe();
        value["methods"][0]["service"]["command"] = json!(["sudo", "demo"]);
        let recipe = parse(value);
        let err = recipe.validate_for_import("linux-x86_64").unwrap_err();
        assert!(err.contains("elevation"), "{err}");
    }

    #[test]
    fn imported_recipe_rejects_unsupported_platform() {
        let recipe = parse(base_recipe());
        let err = recipe.validate_for_import("macos-aarch64").unwrap_err();
        assert!(err.contains("does not support"), "{err}");
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
        let (os, arch) = super::split_platform(platform).unwrap();
        json!({
            "id": "remote-demo",
            "name": "Remote demo",
            "platforms": [platform],
            "methods": [{
                "id": "release",
                "kind": "release",
                "when": { "os": [os], "arch": [arch] },
                "source": {
                    "repo": "owner/repo",
                    "asset": { platform: "demo.tar.gz" }
                },
                "version": { "type": "github-release", "repo": "owner/repo" },
                "service": {
                    "command": ["{install_dir}/bin/demo", "serve"],
                    "health_url": "http://127.0.0.1:9999/health"
                },
                "uninstall": { "keep_paths": [] }
            }]
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
        let url = serve_recipe(remote_recipe(&platform).to_string());
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
        let directory = temp_catalog_dir("roundtrip");
        let recipe = parse(base_recipe());
        let source = RecipeSource {
            url: "https://example.com/easyai.json".to_string(),
            sha256: "abc".to_string(),
            imported_at: "0".to_string(),
        };
        store_imported(&directory, &recipe, &source).unwrap();
        assert_eq!(entries(&directory).unwrap().len(), 2);
        let imported = find(&directory, "demo").unwrap();
        assert_eq!(
            imported.source.as_ref().unwrap().url,
            "https://example.com/easyai.json"
        );
        remove_recipe(&directory, "demo").unwrap();
        assert!(find(&directory, "demo").is_err());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}