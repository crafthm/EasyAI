use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    pub icon: String,
    /// Imported from a repository that publishes no recipe; metadata only.
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub category: Option<Category>,
    #[serde(default)]
    pub methods: Vec<Method>,
    #[serde(default)]
    pub usage: Option<Usage>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    LlmRuntime,
    WebUi,
    ImageGeneration,
    CliTool,
    DesktopApp,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::LlmRuntime => "llm-runtime",
            Category::WebUi => "web-ui",
            Category::ImageGeneration => "image-generation",
            Category::CliTool => "cli-tool",
            Category::DesktopApp => "desktop-app",
        }
    }
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
    #[serde(default)]
    pub usage: Option<Usage>,
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Usage {
    pub ui_url: Option<String>,
    pub website: Option<String>,
    pub api: Option<ApiUsage>,
    pub docs_url: Option<String>,
    #[serde(default)]
    pub cli: Vec<CliUsage>,
    pub launch: Option<LaunchUsage>,
    #[serde(default)]
    pub inferred: bool,
    pub source: Option<String>,
    pub confidence: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ApiUsage {
    pub base_url: Option<String>,
    pub openai_base_url: Option<String>,
    pub docs_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CliUsage {
    #[serde(default)]
    pub id: Option<String>,
    pub label: String,
    pub command: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LaunchUsage {
    pub command: String,
    pub source: Option<String>,
    pub confidence: Option<String>,
}

impl CliUsage {
    pub fn resolved_id(&self) -> String {
        self.id
            .clone()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| format!("cli-{}", slug(&self.label)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageItem {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub value: String,
    pub is_url: bool,
}

fn slug(value: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "item".to_string()
    } else {
        slug
    }
}

impl Usage {
    pub fn validate(&self, recipe_id: &str) -> Result<(), String> {
        for (label, url) in self.urls_with_labels() {
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(format!(
                    "Recipe '{}' usage {label} has an invalid URL '{}'.",
                    recipe_id, url
                ));
            }
        }
        for item in &self.cli {
            if item.label.trim().is_empty() || item.command.trim().is_empty() {
                return Err(format!(
                    "Recipe '{}' has an empty usage CLI entry.",
                    recipe_id
                ));
            }
        }
        let mut ids = HashSet::new();
        for item in self.items() {
            if !ids.insert(item.id.clone()) {
                return Err(format!(
                    "Recipe '{}' has duplicate usage item id '{}'.",
                    recipe_id, item.id
                ));
            }
        }
        Ok(())
    }

    pub fn items(&self) -> Vec<UsageItem> {
        let mut items = Vec::new();
        if let Some(url) = &self.website {
            items.push(UsageItem {
                id: "website".to_string(),
                kind: "website".to_string(),
                label: "Website".to_string(),
                value: url.clone(),
                is_url: true,
            });
        }
        if let Some(url) = &self.ui_url {
            items.push(UsageItem {
                id: "ui".to_string(),
                kind: "web-ui".to_string(),
                label: "Web UI".to_string(),
                value: url.clone(),
                is_url: true,
            });
        }
        if let Some(api) = &self.api {
            if let Some(url) = &api.base_url {
                items.push(UsageItem {
                    id: "api-base".to_string(),
                    kind: "api".to_string(),
                    label: "API base".to_string(),
                    value: url.clone(),
                    is_url: true,
                });
            }
            if let Some(url) = &api.openai_base_url {
                items.push(UsageItem {
                    id: "openai-base".to_string(),
                    kind: "openai".to_string(),
                    label: "OpenAI-compatible".to_string(),
                    value: url.clone(),
                    is_url: true,
                });
            }
            if let Some(url) = &api.docs_url {
                items.push(UsageItem {
                    id: "api-docs".to_string(),
                    kind: "docs".to_string(),
                    label: "API documentation".to_string(),
                    value: url.clone(),
                    is_url: true,
                });
            }
        }
        if let Some(url) = &self.docs_url {
            items.push(UsageItem {
                id: "docs".to_string(),
                kind: "docs".to_string(),
                label: "Documentation".to_string(),
                value: url.clone(),
                is_url: true,
            });
        }
        for cli in &self.cli {
            items.push(UsageItem {
                id: cli.resolved_id(),
                kind: "cli".to_string(),
                label: cli.label.clone(),
                value: cli.command.clone(),
                is_url: false,
            });
        }
        if let Some(launch) = &self.launch {
            items.push(UsageItem {
                id: "launch".to_string(),
                kind: "launch".to_string(),
                label: "Launch".to_string(),
                value: launch.command.clone(),
                is_url: false,
            });
        }
        items
    }

    pub fn merge(defaults: Option<&Usage>, overrides: Option<&Usage>) -> Option<Usage> {
        match (defaults, overrides) {
            (None, None) => None,
            (Some(defaults), None) => Some(defaults.clone()),
            (None, Some(overrides)) => Some(overrides.clone()),
            (Some(defaults), Some(overrides)) => Some(Usage {
                ui_url: overrides.ui_url.clone().or_else(|| defaults.ui_url.clone()),
                website: overrides.website.clone().or_else(|| defaults.website.clone()),
                api: merge_api(defaults.api.as_ref(), overrides.api.as_ref()),
                docs_url: overrides
                    .docs_url
                    .clone()
                    .or_else(|| defaults.docs_url.clone()),
                cli: overrides.cli.clone(),
                launch: overrides.launch.clone().or_else(|| defaults.launch.clone()),
                inferred: overrides.inferred || defaults.inferred,
                source: overrides
                    .source
                    .clone()
                    .or_else(|| defaults.source.clone()),
                confidence: overrides
                    .confidence
                    .clone()
                    .or_else(|| defaults.confidence.clone()),
                notes: overrides.notes.clone().or_else(|| defaults.notes.clone()),
            }),
        }
    }

    pub fn declares(&self, url: &str) -> bool {
        self.declared_urls().iter().any(|declared| declared == url)
    }

    pub fn declared_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        for (_, url) in self.urls_with_labels() {
            if !urls.iter().any(|existing| existing == url) {
                urls.push(url.to_string());
            }
        }
        urls
    }

    fn urls_with_labels(&self) -> Vec<(&'static str, &str)> {
        let mut urls = Vec::new();
        if let Some(url) = &self.ui_url {
            urls.push(("ui_url", url.as_str()));
        }
        if let Some(url) = &self.website {
            urls.push(("website", url.as_str()));
        }
        if let Some(api) = &self.api {
            if let Some(url) = &api.base_url {
                urls.push(("api.base_url", url.as_str()));
            }
            if let Some(url) = &api.openai_base_url {
                urls.push(("api.openai_base_url", url.as_str()));
            }
            if let Some(url) = &api.docs_url {
                urls.push(("api.docs_url", url.as_str()));
            }
        }
        if let Some(url) = &self.docs_url {
            urls.push(("docs_url", url.as_str()));
        }
        urls
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
    /// The platform of the machine running the import, e.g. `linux-x86_64`.
    pub platform: String,
    /// Whether this machine has an install method for the application.
    pub supported: bool,
    /// Every platform the repository publishes for, whether or not this
    /// machine can install it.
    pub detected_platforms: Vec<String>,
    /// Set when the repository does not look like installable software on this
    /// machine; the UI blocks the import and shows `hint`.
    pub unusable: bool,
    pub hint: Option<String>,
    pub conflict: bool,
    pub warnings: Vec<String>,
}

/// Describes why a candidate recipe cannot be used on `platform`, if it cannot.
struct Availability {
    supported: bool,
    detected_platforms: Vec<String>,
    unusable: bool,
    hint: Option<String>,
}

impl Availability {
    fn describe(
        recipe: &Recipe,
        platform: &str,
        analysis: &DraftAnalysis,
    ) -> Result<Self, String> {
        let (os, arch) = split_platform(platform)?;
        let supported = !recipe.methods.is_empty() && recipe.supports_platform(os, arch);
        let detected = if analysis.detected_platforms.is_empty() {
            recipe.platforms.clone()
        } else {
            analysis.detected_platforms.clone()
        };
        if supported {
            return Ok(Self {
                supported,
                detected_platforms: detected,
                unusable: false,
                hint: None,
            });
        }
        if recipe.methods.is_empty() {
            let hint = if !detected.is_empty() {
                format!(
                    "This repository publishes installers for {} but none of them can be installed automatically. It is probably not a runnable application.",
                    summarize_platforms(&detected)
                )
            } else if analysis.installer_only {
                "The latest release only ships native installers, which this version cannot install automatically."
                    .to_string()
            } else {
                "No installable artifact was found. This repository is probably a library, a plugin, or not a runnable application."
                    .to_string()
            };
            return Ok(Self {
                supported,
                detected_platforms: detected,
                unusable: true,
                hint: Some(hint),
            });
        }
        let hint = if detected.is_empty() {
            format!("No install method targets {platform}.")
        } else {
            format!(
                "This application only supports {}, not {platform}.",
                summarize_platforms(&detected)
            )
        };
        Ok(Self {
            supported,
            detected_platforms: detected,
            unusable: true,
            hint: Some(hint),
        })
    }
}

/// Groups platform ids by OS: `linux-x86_64, linux-aarch64, macos-aarch64`
/// becomes `linux, macos`.
fn summarize_platforms(platforms: &[String]) -> String {
    let mut systems: Vec<&str> = platforms
        .iter()
        .filter_map(|platform| platform.split_once('-').map(|(os, _)| os))
        .collect();
    systems.sort_unstable();
    systems.dedup();
    systems.join(", ")
}

fn category_satisfied(category: Category, usage: &Usage) -> bool {
    match category {
        Category::LlmRuntime => {
            usage.api.as_ref().and_then(|api| api.base_url.as_ref()).is_some()
                || !usage.cli.is_empty()
        }
        Category::WebUi => usage.ui_url.is_some(),
        Category::ImageGeneration => {
            usage.ui_url.is_some()
                && usage
                    .notes
                    .as_ref()
                    .map(|notes| !notes.trim().is_empty())
                    .unwrap_or(false)
        }
        Category::CliTool => !usage.cli.is_empty(),
        Category::DesktopApp => usage.ui_url.is_some() || usage.website.is_some(),
    }
}

fn merge_api(defaults: Option<&ApiUsage>, overrides: Option<&ApiUsage>) -> Option<ApiUsage> {
    match (defaults, overrides) {
        (None, None) => None,
        (Some(defaults), None) => Some(defaults.clone()),
        (None, Some(overrides)) => Some(overrides.clone()),
        (Some(defaults), Some(overrides)) => Some(ApiUsage {
            base_url: overrides
                .base_url
                .clone()
                .or_else(|| defaults.base_url.clone()),
            openai_base_url: overrides
                .openai_base_url
                .clone()
                .or_else(|| defaults.openai_base_url.clone()),
            docs_url: overrides
                .docs_url
                .clone()
                .or_else(|| defaults.docs_url.clone()),
        }),
    }
}

impl Recipe {
    pub fn effective_usage(&self, method: &Method) -> Option<Usage> {
        Usage::merge(self.usage.as_ref(), method.usage.as_ref())
    }

    fn validate_category(&self) -> Result<(), String> {
        let Some(category) = self.category else {
            return Ok(());
        };
        let usages: Vec<Usage> = if self.methods.is_empty() {
            self.usage.iter().cloned().collect()
        } else {
            self.methods
                .iter()
                .filter_map(|method| self.effective_usage(method))
                .collect()
        };
        if usages
            .iter()
            .any(|usage| category_satisfied(category, usage))
        {
            return Ok(());
        }
        Err(format!(
            "Recipe '{}' category '{}' is missing required usage.",
            self.id,
            category.as_str()
        ))
    }

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
        if !self.draft && self.platforms.is_empty() {
            return Err(format!(
                "Recipe '{}' does not declare any supported platforms.",
                self.id
            ));
        }
        if !self.draft && self.methods.is_empty() {
            return Err(format!("Recipe '{}' declares no installation methods.", self.id));
        }

        if !self.icon.trim().is_empty() && !self.icon.trim().starts_with("https://") {
            return Err(format!(
                "Recipe '{}' icon must be an https:// URL.",
                self.id
            ));
        }

        if let Some(usage) = &self.usage {
            usage.validate(&self.id)?;
        }

        let mut ids = HashSet::new();
        for method in &self.methods {
            method.validate(&self.id)?;
            if let Some(usage) = &method.usage {
                usage.validate(&self.id)?;
            }
            if let Some(usage) = self.effective_usage(method) {
                usage.validate(&self.id)?;
            }
            if !ids.insert(method.id.clone()) {
                return Err(format!(
                    "Recipe '{}' declares duplicate method id '{}'.",
                    self.id, method.id
                ));
            }
        }
        self.validate_category()?;
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

    /// The tile background: the recipe's own icon when declared, otherwise the
    /// GitHub avatar of the repository the recipe came from.
    pub fn icon_url(&self, source_url: Option<&str>) -> Option<String> {
        let icon = self.icon.trim();
        if !icon.is_empty() {
            return Some(icon.to_string());
        }
        let owner = source_url
            .and_then(github_url_owner)
            .or_else(|| {
                self.methods.iter().find_map(|method| {
                    method
                        .source
                        .as_ref()
                        .and_then(|source| source.repo.as_deref())
                        .or_else(|| {
                            method
                                .version
                                .as_ref()
                                .and_then(|version| version.repo.as_deref())
                        })
                        .and_then(repo_slug_owner)
                })
            })?;
        Some(format!("https://github.com/{owner}.png?size=200"))
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
        imported.sort_by(|left, right| {
            imported_at_secs(left)
                .cmp(&imported_at_secs(right))
                .then_with(|| left.recipe.id.cmp(&right.recipe.id))
        });
        entries.extend(imported);
    }

    Ok(entries)
}

/// Import order key: the epoch seconds recorded when the recipe was imported.
/// Entries without a source (bundled recipes) are never sorted against imports.
fn imported_at_secs(entry: &CatalogEntry) -> u64 {
    entry
        .source
        .as_ref()
        .and_then(|source| source.imported_at.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

pub fn find(app_data: &Path, app_id: &str) -> Result<CatalogEntry, String> {
    entries(app_data)?
        .into_iter()
        .find(|entry| entry.recipe.id == app_id)
        .ok_or_else(|| format!("Unknown application '{app_id}'."))
}

pub fn preview_recipe(app_data: &Path, input: &str) -> Result<RecipePreview, String> {
    let platform = crate::probe::current_platform_id()?;
    let Some((source_url, content)) = try_fetch_recipe(input)? else {
        return preview_draft(app_data, input, &platform);
    };
    let sha256 = sha256_hex(content.as_bytes());
    let recipe = parse_recipe(&content)?;
    let availability = Availability::describe(&recipe, &platform, &DraftAnalysis::default())?;
    let conflict = entries(app_data)?
        .iter()
        .any(|entry| entry.recipe.id == recipe.id);

    let mut warnings = vec![
        "Release archives are not checksum-verified by this version.".to_string(),
    ];
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
        supported: availability.supported,
        detected_platforms: availability.detected_platforms,
        unusable: availability.unusable,
        hint: availability.hint,
        conflict,
        warnings,
    })
}

fn preview_draft(
    app_data: &Path,
    input: &str,
    platform: &str,
) -> Result<RecipePreview, String> {
    let (owner, repo_name) = github_repo_from_input(input)?;
    let repo = crate::github::repo_summary(&owner, &repo_name)?;
    let (release, release_error) = match crate::github::latest_release(&owner, &repo_name) {
        Ok(release) => (release, None),
        Err(err) => (None, Some(err)),
    };
    let inference = release.as_ref().map(|release| {
        crate::usage_infer::infer_release(
            &owner,
            &repo.name,
            repo.description.as_deref(),
            release,
            platform,
        )
    });
    let (recipe, candidate_warnings, analysis) =
        candidate_recipe(&owner, &repo, release.as_ref(), inference.as_ref());
    let mut warnings = vec![
        "This repository publishes no EasyAI recipe. A candidate recipe was generated from its latest release."
            .to_string(),
    ];
    warnings.extend(candidate_warnings);
    if let Some(err) = release_error {
        warnings.push(format!("Could not read the latest release: {err}"));
    }
    let sha256 = sha256_hex(&draft_bytes(&recipe)?);
    let conflict = entries(app_data)?
        .iter()
        .any(|entry| entry.recipe.id == recipe.id);
    let availability = Availability::describe(&recipe, platform, &analysis)?;
    if conflict {
        warnings.push(format!(
            "An application with id '{}' already exists.",
            recipe.id
        ));
    }

    Ok(RecipePreview {
        recipe,
        source_url: repo.html_url,
        sha256,
        platform: platform.to_string(),
        supported: availability.supported,
        detected_platforms: availability.detected_platforms,
        unusable: availability.unusable,
        hint: availability.hint,
        conflict,
        warnings,
    })
}

fn import_draft(
    app_data: &Path,
    input: &str,
    expected_sha256: &str,
) -> Result<CatalogEntry, String> {
    let (owner, repo_name) = github_repo_from_input(input)?;
    let repo = crate::github::repo_summary(&owner, &repo_name)?;
    let release = crate::github::latest_release(&owner, &repo_name)?;
    let platform = crate::probe::current_platform_id()?;
    let inference = release.as_ref().map(|release| {
        crate::usage_infer::infer_release(
            &owner,
            &repo.name,
            repo.description.as_deref(),
            release,
            &platform,
        )
    });
    let (recipe, _, analysis) =
        candidate_recipe(&owner, &repo, release.as_ref(), inference.as_ref());
    let sha256 = sha256_hex(&draft_bytes(&recipe)?);
    if sha256 != expected_sha256 {
        return Err(
            "The repository changed since it was previewed. Preview it again.".to_string(),
        );
    }
    if entries(app_data)?
        .iter()
        .any(|entry| entry.recipe.id == recipe.id)
    {
        return Err(format!(
            "An application with id '{}' already exists.",
            recipe.id
        ));
    }
    let availability = Availability::describe(&recipe, &platform, &analysis)?;
    if availability.unusable {
        return Err(availability
            .hint
            .unwrap_or_else(|| format!("This repository cannot be installed on {platform}.")));
    }

    let source = RecipeSource {
        url: repo.html_url,
        sha256,
        imported_at: imported_at(),
    };
    store_imported(app_data, &recipe, &source)?;
    Ok(CatalogEntry {
        recipe,
        source: Some(source),
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
    let Some((source_url, content)) = try_fetch_recipe(input)? else {
        return import_draft(app_data, input, expected_sha256);
    };
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

    let imported_at = imported_at();
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

pub fn update_imported_recipe(
    app_data: &Path,
    recipe: &Recipe,
) -> Result<CatalogEntry, String> {
    recipe.validate()?;
    let path = imported_path(app_data, &recipe.id);
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read the imported recipe: {err}"))?;
    let stored: StoredRecipe = serde_json::from_str(&raw)
        .map_err(|err| format!("Failed to parse the imported recipe: {err}"))?;
    if stored.recipe.id != recipe.id {
        return Err("The imported recipe id does not match.".to_string());
    }
    store_imported(app_data, recipe, &stored.source)?;
    Ok(CatalogEntry {
        recipe: recipe.clone(),
        source: Some(stored.source),
    })
}

pub fn parse_github_repo_url(input: &str) -> Option<(String, String)> {
    let url = reqwest::Url::parse(input).ok()?;
    github_repo_parts(&url)
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

/// Returns the recipe at this URL. `None` means the input is a GitHub
/// repository that publishes no recipe; the caller may import a draft instead.
fn try_fetch_recipe(input: &str) -> Result<Option<(String, String)>, String> {
    let url = reqwest::Url::parse(input.trim())
        .map_err(|_| "Enter a valid recipe or GitHub repository URL.".to_string())?;
    ensure_allowed_scheme(&url)?;

    if let Some((owner, repo)) = github_repo_parts(&url) {
        let candidates = [
            format!("https://raw.githubusercontent.com/{owner}/{repo}/HEAD/easyai.json"),
            format!("https://raw.githubusercontent.com/{owner}/{repo}/HEAD/.easyai/recipe.json"),
        ];
        for candidate in candidates {
            if let Some(content) = http_get_optional(&candidate)? {
                return Ok(Some((candidate, content)));
            }
        }
        return Ok(None);
    }

    let content = http_get(url.as_str())?;
    Ok(Some((url.to_string(), content)))
}

/// Builds a candidate recipe from repository metadata and its latest release.
/// Without a usable archive the recipe stays a metadata-only draft.
fn candidate_recipe(
    owner: &str,
    repo: &crate::github::RepoSummary,
    release: Option<&crate::github::ReleaseInfo>,
    inference: Option<&crate::usage_infer::Inference>,
) -> (Recipe, Vec<String>, DraftAnalysis) {
    let full_repo = format!("{owner}/{}", repo.name);
    let mut assets: BTreeMap<String, (u8, String)> = BTreeMap::new();
    let mut detected: BTreeSet<String> = BTreeSet::new();
    let mut installer_only = false;
    if let Some(release) = release {
        for asset in &release.assets {
            let Some((rank, packaging)) = asset_rank(&asset.name) else {
                continue;
            };
            let platforms = platforms_for_asset(&asset.name);
            detected.extend(platforms.iter().cloned());
            if packaging != Packaging::Archive {
                installer_only = true;
                continue;
            }
            for platform in platforms {
                if assets
                    .get(&platform)
                    .is_some_and(|(best, _)| *best <= rank)
                {
                    continue;
                }
                assets.insert(
                    platform,
                    (rank, versioned_asset(&asset.name, &release.tag_name)),
                );
            }
        }
    }

    let methods: Vec<Method> = assets
        .iter()
        .map(|(platform, (_, asset))| release_method(&full_repo, platform, asset))
        .collect();
    let mut warnings = Vec::new();
    if methods.is_empty() {
        warnings.push(
            "No usable release archive was found. The app will be added as a draft without an install method."
                .to_string(),
        );
        if installer_only {
            warnings.push(
                "The latest release only ships native installers, which this version cannot install automatically."
                    .to_string(),
            );
        }
    } else {
        warnings.push(
            "Install method inferred from the latest GitHub Release. Review it before installing."
                .to_string(),
        );
        if inference.and_then(|inference| inference.launch.as_ref()).is_none() {
            warnings.push(
                "No start command was inferred. Start/Stop will not work until the recipe is edited."
                    .to_string(),
            );
        }
    }

    let mut usage = repo.homepage.as_ref().map(|homepage| Usage {
        website: Some(homepage.clone()),
        ..Default::default()
    });
    let category = inference.and_then(|inference| inference.category);
    if let Some(inference) = inference {
        if inference.launch.is_some()
            || !inference.cli.is_empty()
            || inference.category.is_some()
        {
            let usage = usage.get_or_insert_with(Usage::default);
            if let Some(launch) = &inference.launch {
                usage.launch = Some(launch.clone());
            }
            usage.cli = inference.cli.clone();
            usage.inferred = true;
            usage.source = Some(inference.source.clone());
            usage.confidence = Some(inference.confidence.clone());
        }
        warnings.extend(inference.warnings.clone());
    }

    let recipe = Recipe {
        id: draft_id(owner, &repo.name),
        name: repo.name.clone(),
        description: repo.description.clone().unwrap_or_default(),
        glyph: String::new(),
        color: String::new(),
        icon: repo.logo.clone().unwrap_or_default(),
        draft: methods.is_empty(),
        platforms: assets.keys().cloned().collect(),
        category,
        methods,
        usage,
        legacy_install: None,
        legacy_service: None,
        legacy_uninstall: None,
    };
    let analysis = DraftAnalysis {
        detected_platforms: detected.into_iter().collect(),
        installer_only,
    };
    (recipe, warnings, analysis)
}

/// What the release assets reveal beyond the generated recipe: every platform
/// the repository publishes for, and whether it only ships native installers.
#[derive(Debug, Clone, Default)]
struct DraftAnalysis {
    detected_platforms: Vec<String>,
    installer_only: bool,
}

fn release_method(repo: &str, platform: &str, asset: &str) -> Method {
    let (os, arch) = platform.split_once('-').unwrap_or((platform, ""));
    Method {
        id: format!("release-{platform}"),
        kind: MethodKind::Release,
        when: MethodWhen {
            os: vec![os.to_string()],
            arch: vec![arch.to_string()],
            ..Default::default()
        },
        prefer: 80,
        advanced: false,
        elevation: false,
        version: Some(VersionSource {
            kind: "github-release".to_string(),
            repo: Some(repo.to_string()),
            command: None,
            package: None,
            image: None,
        }),
        service: None,
        uninstall: Some(Uninstall {
            keep_paths: Vec::new(),
            remove_paths: Vec::new(),
            command: None,
            native: false,
        }),
        size: None,
        source: Some(MethodSource {
            repo: Some(repo.to_string()),
            asset: HashMap::from([(platform.to_string(), asset.to_string())]),
            url: None,
            sha256: None,
            git: None,
            git_ref: None,
            build: None,
        }),
        image: None,
        package: None,
        language: None,
        script: None,
        universal: None,
        usage: None,
    }
}

/// How a release asset is packaged, which decides both its platform and
/// whether the release executor can extract it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Packaging {
    /// An archive the executor can unpack into an install directory.
    Archive,
    /// A native installer or package that needs the OS package handler.
    Installer,
}

/// Whether an asset is something a user could install, rather than a checksum,
/// signature, debug symbols, or a source tarball published alongside it.
fn is_distributable(lower: &str) -> bool {
    !(lower.ends_with(".sha256")
        || lower.contains("sha256sum")
        || lower.ends_with(".sig")
        || lower.ends_with(".asc")
        || lower.ends_with(".zsync")
        || lower.contains("delta")
        || lower.contains("sources")
        || lower.contains("-source")
        || lower.contains("_source")
        || lower.contains("symbols")
        || lower.contains("debug")
        || lower.contains("checksum"))
}

/// Downloadable release formats, best first. Archives rank before native
/// installers because they can be installed without elevation.
fn asset_rank(name: &str) -> Option<(u8, Packaging)> {
    let lower = name.to_ascii_lowercase();
    if !is_distributable(&lower) {
        return None;
    }
    let archive = if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some(0)
    } else if lower.ends_with(".tar.zst") {
        Some(1)
    } else if lower.ends_with(".zip") {
        Some(2)
    } else if lower.ends_with(".7z") {
        Some(3)
    } else if lower.ends_with(".tar") {
        Some(4)
    } else {
        None
    };
    if let Some(rank) = archive {
        return Some((rank, Packaging::Archive));
    }
    const INSTALLERS: [&str; 8] = [
        ".deb", ".rpm", ".appimage", ".snap", ".exe", ".msi", ".dmg", ".pkg",
    ];
    INSTALLERS
        .iter()
        .any(|suffix| lower.ends_with(suffix))
        .then_some((5, Packaging::Installer))
}

/// The operating system an asset targets, when it can be determined from the
/// file name. Installer formats are decided by their extension first, so a
/// `.dmg` or a `.exe` needs no `macos`/`windows` token in its name.
fn asset_os(lower: &str) -> Option<&'static str> {
    const FORMATS: [(&str, &str); 11] = [
        (".deb", "linux"),
        (".rpm", "linux"),
        (".appimage", "linux"),
        (".snap", "linux"),
        (".flatpak", "linux"),
        (".exe", "windows"),
        (".msi", "windows"),
        (".msix", "windows"),
        (".dmg", "macos"),
        (".pkg", "macos"),
        (".app", "macos"),
    ];
    if let Some((_, os)) = FORMATS
        .iter()
        .find(|(suffix, _)| lower.ends_with(suffix))
    {
        return Some(os);
    }
    const TOKENS: [(&str, &str); 11] = [
        ("linux", "linux"),
        ("gnu", "linux"),
        ("musl", "linux"),
        ("windows", "windows"),
        ("win32", "windows"),
        ("win64", "windows"),
        ("mingw", "windows"),
        ("msvc", "windows"),
        ("macos", "macos"),
        ("darwin", "macos"),
        ("osx", "macos"),
    ];
    TOKENS
        .iter()
        .find(|(token, _)| lower.contains(token))
        .map(|(_, os)| *os)
}

/// Architectures an asset targets. A macOS `universal` build covers both.
fn asset_archs(lower: &str) -> Vec<&'static str> {
    if lower.contains("universal") {
        return vec!["x86_64", "aarch64"];
    }
    if lower.contains("aarch64") || lower.contains("arm64") {
        return vec!["aarch64"];
    }
    // armv7 and 32-bit x86 are outside the supported platforms.
    if lower.contains("armv7") || lower.contains("armhf") {
        return Vec::new();
    }
    // An asset without an architecture token is treated as the common case.
    vec!["x86_64"]
}

fn platforms_for_asset(name: &str) -> Vec<String> {
    let lower = name.to_ascii_lowercase();
    if !is_distributable(&lower) {
        return Vec::new();
    }
    let Some(os) = asset_os(&lower) else {
        return Vec::new();
    };
    asset_archs(&lower)
        .into_iter()
        .map(|arch| format!("{os}-{arch}"))
        .collect()
}

/// Replaces the concrete version in an asset name with the `{version}`
/// template the release executor expands at install time.
fn versioned_asset(name: &str, tag: &str) -> String {
    let version = tag.trim().trim_start_matches(['v', 'V']);
    if version.len() < 3 || !version.contains('.') {
        return name.to_string();
    }
    name.replace(version, "{version}")
}

/// A stable id for a draft: `owner-repo` lowercased and limited to the
/// characters allowed in recipe ids.
fn draft_id(owner: &str, name: &str) -> String {
    format!("{owner}-{name}")
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn github_repo_from_input(input: &str) -> Result<(String, String), String> {
    let url = reqwest::Url::parse(input.trim())
        .map_err(|_| "Enter a valid GitHub repository URL.".to_string())?;
    github_repo_parts(&url)
        .ok_or_else(|| "Enter a valid GitHub repository URL.".to_string())
}

fn draft_bytes(recipe: &Recipe) -> Result<Vec<u8>, String> {
    serde_json::to_vec(recipe).map_err(|err| format!("Failed to prepare the draft: {err}"))
}

fn imported_at() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
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

fn github_url_owner(url: &str) -> Option<String> {
    let url = reqwest::Url::parse(url).ok()?;
    let owner = match url.host_str()? {
        "github.com" | "raw.githubusercontent.com" => url
            .path_segments()?
            .find(|segment| !segment.is_empty())?
            .to_string(),
        _ => return None,
    };
    is_valid_github_name(&owner).then_some(owner)
}

/// `owner/repo` as written in a recipe's `source` or `version` section.
fn repo_slug_owner(slug: &str) -> Option<String> {
    let (owner, repo) = slug.split_once('/')?;
    (is_valid_github_name(owner) && is_valid_github_name(repo)).then(|| owner.to_string())
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
    http_get_optional(url)?.ok_or_else(|| "Recipe not found (404).".to_string())
}

fn http_get_optional(url: &str) -> Result<Option<String>, String> {
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
    if status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(match status.as_u16() {
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
    let content = String::from_utf8(buffer)
        .map_err(|_| "The recipe is not valid UTF-8.".to_string())?;
    Ok(Some(content))
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
        assert_eq!(
            recipes[0].icon_url(None).as_deref(),
            Some("https://github.com/ollama.png?size=200")
        );
        let usage = recipes[0].usage.as_ref().unwrap();
        assert!(usage.declares("http://127.0.0.1:11434"));
        assert!(usage.declares("http://127.0.0.1:11434/v1"));
        assert_eq!(usage.cli.len(), 3);
        assert_eq!(recipes[0].category, Some(super::Category::LlmRuntime));
        let container = recipes[0].method("container").unwrap();
        let container_usage = recipes[0].effective_usage(container).unwrap();
        assert!(container_usage.cli.is_empty());
        assert_eq!(
            container_usage.api.unwrap().base_url.as_deref(),
            Some("http://127.0.0.1:11434")
        );
    }

    #[test]
    fn valid_recipe_passes() {
        let recipe = parse(base_recipe());
        assert!(recipe.validate().is_ok());
        assert_eq!(recipe.methods.len(), 1);
    }

    #[test]
    fn draft_recipe_validates_without_methods_or_platforms() {
        let draft = parse(json!({
            "id": "owner-repo",
            "name": "repo",
            "draft": true
        }));
        assert!(draft.validate().is_ok());
        assert_eq!(
            draft
                .icon_url(Some("https://github.com/owner/repo"))
                .as_deref(),
            Some("https://github.com/owner.png?size=200")
        );

        let regular = parse(json!({ "id": "owner-repo", "name": "repo" }));
        assert!(regular.validate().is_err());
    }

    #[test]
    fn draft_ids_are_lowercase_and_url_safe() {
        assert_eq!(
            super::draft_id("Tangency-Labs", "Solid.Craft"),
            "tangency-labs-solid-craft"
        );
    }

    #[test]
    fn asset_rank_skips_deltas_and_sources() {
        assert!(super::asset_rank("BeefTV-v2.1.2-linux-amd64.zip").is_some());
        assert!(
            super::asset_rank("BeefTV-v2.1.2-linux-amd64-from-v2.1.1.delta.zip").is_none()
        );
        assert!(super::asset_rank(
            "BeefTV-v2.1.2-ffprobe-darwin-amd64-sources.tar.gz"
        )
        .is_none());
        assert!(super::asset_rank("app-linux-amd64.sha256").is_none());
    }

    #[test]
    fn installer_formats_decide_the_platform() {
        // A `.dmg` or `.exe` needs no platform token in its name.
        assert_eq!(
            super::platforms_for_asset("SolidCraft-0.27.1.dmg"),
            vec!["macos-x86_64"]
        );
        assert_eq!(
            super::platforms_for_asset("SolidCraft-Setup.exe"),
            vec!["windows-x86_64"]
        );
        assert_eq!(
            super::platforms_for_asset("solidcraft_0.27.1_amd64.deb"),
            vec!["linux-x86_64"]
        );
        assert_eq!(
            super::platforms_for_asset("SolidCraft-0.27.1.AppImage"),
            vec!["linux-x86_64"]
        );
        assert_eq!(
            super::platforms_for_asset("SolidCraft-0.27.1-aarch64.dmg"),
            vec!["macos-aarch64"]
        );
        assert_eq!(
            super::platforms_for_asset("SolidCraft-0.27.1-macos-universal.dmg"),
            vec!["macos-x86_64", "macos-aarch64"]
        );
        // Sources, checksums, and unrelated files carry no platform.
        assert!(super::platforms_for_asset("SHA256SUMS.txt").is_empty());
        assert!(super::platforms_for_asset("app-linux-amd64.sha256").is_empty());
        // armv7 is outside the supported platforms.
        assert!(super::platforms_for_asset("app-linux-armv7.tar.gz").is_empty());
    }

    #[test]
    fn installer_only_release_is_reported_as_unusable() {
        let repo = crate::github::RepoSummary {
            name: "solidcraft".to_string(),
            html_url: "https://github.com/tangency-labs/solidcraft".to_string(),
            description: Some("CAD".to_string()),
            homepage: None,
            logo: None,
        };
        let release = crate::github::ReleaseInfo {
            tag_name: "v1.0.0".to_string(),
            assets: vec![crate::github::ReleaseAsset {
                name: "SolidCraft-1.0.0.dmg".to_string(),
                size: 0,
            }],
        };
        let (recipe, _, analysis) =
            super::candidate_recipe("tangency-labs", &repo, Some(&release), None);
        assert!(recipe.draft);
        assert!(analysis.installer_only);
        assert_eq!(analysis.detected_platforms, vec!["macos-x86_64"]);

        let availability =
            super::Availability::describe(&recipe, "linux-x86_64", &analysis).unwrap();
        assert!(!availability.supported);
        assert!(availability.unusable);
        assert!(
            availability
                .hint
                .unwrap()
                .contains("macos"),
            "the hint should name the platforms the release publishes for"
        );
    }

    #[test]
    fn repository_without_assets_is_unusable() {
        let recipe = parse(json!({
            "id": "owner-repo",
            "name": "repo",
            "draft": true
        }));
        let availability = super::Availability::describe(
            &recipe,
            "linux-x86_64",
            &super::DraftAnalysis::default(),
        )
        .unwrap();
        assert!(availability.unusable);
        assert!(availability.detected_platforms.is_empty());
    }

    #[test]
    fn supported_recipe_is_usable() {
        let recipe = parse(base_recipe());
        let availability =
            super::Availability::describe(&recipe, "linux-x86_64", &super::DraftAnalysis::default())
                .unwrap();
        assert!(availability.supported);
        assert!(!availability.unusable);
        assert!(availability.hint.is_none());
    }

    #[test]
    fn other_platform_recipe_reports_a_hint() {
        let recipe = parse(base_recipe());
        let availability =
            super::Availability::describe(&recipe, "macos-aarch64", &super::DraftAnalysis::default())
                .unwrap();
        assert!(!availability.supported);
        assert!(availability.unusable);
        assert!(availability.hint.unwrap().contains("linux"));
    }

    #[test]
    fn candidate_recipe_infers_release_methods_from_assets() {
        let repo = crate::github::RepoSummary {
            name: "solidcraft".to_string(),
            html_url: "https://github.com/tangency-labs/solidcraft".to_string(),
            description: Some("CAD".to_string()),
            homepage: Some("https://solidcraft.example".to_string()),
            logo: Some(
                "https://raw.githubusercontent.com/tangency-labs/solidcraft/main/assets/app-icon.png"
                    .to_string(),
            ),
        };
        let asset = |name: &str| crate::github::ReleaseAsset {
            name: name.to_string(),
            size: 0,
        };
        let release = crate::github::ReleaseInfo {
            tag_name: "v0.27.1".to_string(),
            assets: vec![
                asset("solidcraft-0.27.1-linux-x86_64.tar.gz"),
                asset("solidcraft-0.27.1-linux-aarch64.tar.gz"),
                asset("solidcraft-0.27.1-macos-universal.dmg"),
                asset("SHA256SUMS.txt"),
            ],
        };

        let (recipe, warnings, analysis) =
            super::candidate_recipe("tangency-labs", &repo, Some(&release), None);
        assert!(!recipe.draft);
        assert_eq!(recipe.id, "tangency-labs-solidcraft");
        assert_eq!(recipe.platforms, vec!["linux-aarch64", "linux-x86_64"]);
        assert_eq!(recipe.methods.len(), 2);
        let method = recipe
            .methods
            .iter()
            .find(|method| method.id == "release-linux-x86_64")
            .unwrap();
        assert_eq!(
            method.source.as_ref().unwrap().asset["linux-x86_64"],
            "solidcraft-{version}-linux-x86_64.tar.gz"
        );
        // The macOS asset is a native installer: it counts as platform evidence
        // but does not produce an install method this version can execute.
        assert_eq!(
            analysis.detected_platforms,
            vec!["linux-aarch64", "linux-x86_64", "macos-aarch64", "macos-x86_64"]
        );
        assert!(analysis.installer_only);
        assert!(recipe.validate().is_ok());
        assert!(warnings.iter().any(|warning| warning.contains("No start command")));
    }

    #[test]
    fn icon_resolves_from_recipe_icon_or_source_owner() {
        let mut value = base_recipe();
        let recipe = parse(value.clone());
        assert_eq!(
            recipe.icon_url(None).as_deref(),
            Some("https://github.com/owner.png?size=200")
        );
        assert_eq!(
            recipe
                .icon_url(Some(
                    "https://raw.githubusercontent.com/acme/demo/HEAD/easyai.json"
                ))
                .as_deref(),
            Some("https://github.com/acme.png?size=200")
        );

        value["icon"] = json!("https://example.com/icon.png");
        assert_eq!(
            parse(value.clone()).icon_url(None).as_deref(),
            Some("https://example.com/icon.png")
        );

        value["icon"] = json!("http://example.com/icon.png");
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("icon must be an https:// URL"), "{err}");
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
    fn usage_items_include_the_launch_command() {
        let usage: super::Usage = serde_json::from_value(json!({
            "launch": {
                "command": "{install_dir}/bin/demo",
                "source": "artifact",
                "confidence": "high"
            },
            "inferred": true,
            "source": "artifact",
            "confidence": "high"
        }))
        .unwrap();
        assert!(usage.inferred);
        assert_eq!(usage.source.as_deref(), Some("artifact"));
        assert_eq!(usage.confidence.as_deref(), Some("high"));
        let items = usage.items();
        assert!(items
            .iter()
            .any(|item| item.id == "launch" && item.kind == "launch"));
        usage.validate("demo").unwrap();
    }

    #[test]
    fn candidate_recipe_uses_inferred_category_and_launch() {
        let repo = crate::github::RepoSummary {
            name: "BeefTV".to_string(),
            html_url: "https://github.com/glanderness/BeefTV".to_string(),
            description: Some("AI Native Video workspace".to_string()),
            homepage: Some("https://beeftv.app/".to_string()),
            logo: None,
        };
        let release = crate::github::ReleaseInfo {
            tag_name: "v2.1.6".to_string(),
            assets: vec![crate::github::ReleaseAsset {
                name: "BeefTV-v2.1.6-linux-amd64.zip".to_string(),
                size: 10,
            }],
        };
        let inference = crate::usage_infer::Inference {
            category: Some(super::Category::DesktopApp),
            launch: Some(super::LaunchUsage {
                command: "{install_dir}/BeefTV-linux/BeefTV".to_string(),
                source: Some("artifact".to_string()),
                confidence: Some("high".to_string()),
            }),
            cli: vec![super::CliUsage {
                id: Some("cli-help".to_string()),
                label: "CLI help".to_string(),
                command: "{install_dir}/BeefTV-linux/cli/beeftv --help".to_string(),
            }],
            warnings: Vec::new(),
            source: "artifact".to_string(),
            confidence: "high".to_string(),
        };
        let (recipe, warnings, _) = super::candidate_recipe(
            "glanderness",
            &repo,
            Some(&release),
            Some(&inference),
        );
        assert_eq!(recipe.category, Some(super::Category::DesktopApp));
        let usage = recipe.usage.as_ref().unwrap();
        assert!(usage.inferred);
        assert_eq!(
            usage.launch.as_ref().unwrap().command,
            "{install_dir}/BeefTV-linux/BeefTV"
        );
        assert_eq!(usage.cli.len(), 1);
        assert_eq!(usage.cli[0].label, "CLI help");
        assert!(!warnings.iter().any(|warning| warning.contains("No start command")));
    }

    #[test]
    fn update_imported_recipe_preserves_source() {
        let directory = temp_catalog_dir("update-import");
        let mut recipe = parse(base_recipe());
        recipe.usage = Some(super::Usage {
            website: Some("https://example.com".to_string()),
            ..Default::default()
        });
        let source = RecipeSource {
            url: "https://github.com/owner/repo".to_string(),
            sha256: "abc".to_string(),
            imported_at: "0".to_string(),
        };
        store_imported(&directory, &recipe, &source).unwrap();
        recipe.category = Some(super::Category::DesktopApp);
        recipe.usage.as_mut().unwrap().launch = Some(super::LaunchUsage {
            command: "{install_dir}/demo".to_string(),
            source: Some("artifact".to_string()),
            confidence: Some("high".to_string()),
        });
        let updated = super::update_imported_recipe(&directory, &recipe).unwrap();
        assert_eq!(
            updated.source.as_ref().unwrap().url,
            "https://github.com/owner/repo"
        );
        let stored = find(&directory, "demo").unwrap();
        assert_eq!(stored.recipe.category, Some(super::Category::DesktopApp));
        assert!(stored
            .recipe
            .usage
            .as_ref()
            .and_then(|usage| usage.launch.as_ref())
            .is_some());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn imported_entries_follow_creation_order() {
        let directory = temp_catalog_dir("creation-order");
        let source = |imported_at: &str| RecipeSource {
            url: "https://example.com/easyai.json".to_string(),
            sha256: "abc".to_string(),
            imported_at: imported_at.to_string(),
        };
        // Ids are deliberately in reverse alphabetical order so that an
        // id-based sort would produce the opposite sequence.
        let mut newer = parse(base_recipe());
        newer.id = "aaa-newer".to_string();
        newer.name = "Newer".to_string();
        let mut older = parse(base_recipe());
        older.id = "zzz-older".to_string();
        older.name = "Older".to_string();
        store_imported(&directory, &newer, &source("200")).unwrap();
        store_imported(&directory, &older, &source("100")).unwrap();

        let ids: Vec<String> = entries(&directory)
            .unwrap()
            .into_iter()
            .map(|entry| entry.recipe.id)
            .collect();
        assert_eq!(ids, vec!["ollama", "zzz-older", "aaa-newer"]);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn merges_method_usage_with_recipe_defaults() {
        let mut value = base_recipe();
        value["usage"] = json!({
            "ui_url": "http://127.0.0.1:8080",
            "api": { "base_url": "http://127.0.0.1:8080/api" },
            "cli": [
                { "id": "cli-run", "label": "Run", "command": "demo run" }
            ]
        });
        value["methods"][0]["usage"] = json!({
            "ui_url": "http://127.0.0.1:3000",
            "cli": []
        });
        let recipe = parse(value);
        let method = recipe.method("release").unwrap();
        let usage = recipe.effective_usage(method).unwrap();
        assert_eq!(usage.ui_url.as_deref(), Some("http://127.0.0.1:3000"));
        assert_eq!(
            usage.api.as_ref().unwrap().base_url.as_deref(),
            Some("http://127.0.0.1:8080/api")
        );
        assert!(usage.cli.is_empty());
    }

    #[test]
    fn usage_items_have_stable_ids_and_reject_duplicates() {
        let usage: super::Usage = serde_json::from_value(json!({
            "cli": [
                { "label": "Pull a model", "command": "demo pull" },
                { "id": "custom", "label": "Run", "command": "demo run" }
            ]
        }))
        .unwrap();
        let items = usage.items();
        assert_eq!(items[0].id, "cli-pull-a-model");
        assert_eq!(items[1].id, "custom");
        usage.validate("demo").unwrap();

        let duplicate: super::Usage = serde_json::from_value(json!({
            "cli": [
                { "id": "same", "label": "One", "command": "demo one" },
                { "id": "same", "label": "Two", "command": "demo two" }
            ]
        }))
        .unwrap();
        let err = duplicate.validate("demo").unwrap_err();
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn category_requirements_are_enforced() {
        let cases = [
            (
                "llm-runtime",
                json!({ "api": { "base_url": "http://127.0.0.1:11434" } }),
                true,
            ),
            ("llm-runtime", json!({}), false),
            (
                "web-ui",
                json!({ "ui_url": "http://127.0.0.1:3000" }),
                true,
            ),
            ("web-ui", json!({}), false),
            (
                "image-generation",
                json!({ "ui_url": "http://127.0.0.1:8188", "notes": "Models go in models/." }),
                true,
            ),
            (
                "image-generation",
                json!({ "ui_url": "http://127.0.0.1:8188" }),
                false,
            ),
            (
                "cli-tool",
                json!({ "cli": [{ "id": "cli-run", "label": "Run", "command": "demo run" }] }),
                true,
            ),
            ("cli-tool", json!({}), false),
            (
                "desktop-app",
                json!({ "website": "https://example.com" }),
                true,
            ),
            ("desktop-app", json!({}), false),
        ];
        for (category, usage, valid) in cases {
            let mut value = base_recipe();
            value["category"] = json!(category);
            value["usage"] = usage;
            let result = parse(value).validate();
            assert_eq!(result.is_ok(), valid, "category {category}: {result:?}");
        }
    }

    #[test]
    fn usage_urls_are_validated_and_deduplicated() {
        let mut value = base_recipe();
        value["usage"] = json!({
            "ui_url": "http://127.0.0.1:3000",
            "api": {
                "base_url": "http://127.0.0.1:11434",
                "openai_base_url": "http://127.0.0.1:11434/v1",
                "docs_url": "https://docs.example.com"
            },
            "docs_url": "https://docs.example.com",
            "cli": [
                { "label": "Run", "command": "demo run" }
            ],
            "notes": "Open the web UI."
        });
        let recipe = parse(value.clone());
        recipe.validate().unwrap();
        let urls = recipe.usage.as_ref().unwrap().declared_urls();
        assert_eq!(urls.len(), 4);
        assert!(urls.contains(&"http://127.0.0.1:3000".to_string()));
        assert!(urls.contains(&"https://docs.example.com".to_string()));
        assert!(recipe.usage.as_ref().unwrap().declares("http://127.0.0.1:11434"));
        assert!(!recipe
            .usage
            .as_ref()
            .unwrap()
            .declares("https://example.com/other"));

        value["usage"]["ui_url"] = json!("ftp://example.com");
        let err = parse(value).validate().unwrap_err();
        assert!(err.contains("invalid URL"), "{err}");
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