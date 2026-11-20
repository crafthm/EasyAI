use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::catalog::{self, Recipe, RecipeSource};
use crate::db;
use crate::probe;

const BUNDLE_VERSION: u32 = 1;
/// Keeps a hand-edited bundle from consuming unbounded memory.
const MAX_BUNDLE_BYTES: usize = 4 * 1024 * 1024;

/// Portable form of the installed catalog: one `easyai.json` that another
/// machine can import and install from.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bundle {
    pub version: u32,
    pub apps: Vec<BundleApp>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleApp {
    pub recipe: Recipe,
    /// Original provenance, kept so the imported entry can still resolve its
    /// icon and repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<RecipeSource>,
    /// What was installed when the file was exported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<BundleInstall>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleInstall {
    pub version: String,
    #[serde(default)]
    pub method_id: Option<String>,
}

/// What the import dialog shows for one application.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleEntry {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub method_id: Option<String>,
    /// `install`, `installed`, or `unsupported`.
    pub status: String,
    pub message: Option<String>,
}

pub fn export(db_path: &Path, app_data: &Path) -> Result<String, String> {
    let entries = catalog::entries(app_data)?;
    let installed = db::list_installed(db_path)
        .map_err(|err| format!("Failed to load installed applications: {err}"))?;

    let mut apps = Vec::with_capacity(installed.len());
    for record in installed {
        let entry = entries
            .iter()
            .find(|entry| entry.recipe.id == record.app_id)
            .ok_or_else(|| {
                format!(
                    "Cannot export '{}': its recipe is missing from the catalog.",
                    record.app_id
                )
            })?;
        apps.push(BundleApp {
            recipe: entry.recipe.clone(),
            source: entry.source.clone(),
            installed: Some(BundleInstall {
                version: record.version,
                method_id: record.method_id,
            }),
        });
    }

    if apps.is_empty() {
        return Err("No installed applications to export.".to_string());
    }

    let mut text = serde_json::to_string_pretty(&Bundle {
        version: BUNDLE_VERSION,
        apps,
    })
    .map_err(|err| format!("Failed to serialize the catalog: {err}"))?;
    text.push('\n');
    Ok(text)
}

pub fn preview(db_path: &Path, app_data: &Path, raw: &str) -> Result<Vec<BundleEntry>, String> {
    let bundle = parse(raw)?;
    analyze(db_path, app_data, &bundle)
        .map(|items| items.into_iter().map(|item| item.entry).collect())
}

pub fn import(db_path: &Path, app_data: &Path, raw: &str) -> Result<Vec<BundleEntry>, String> {
    let bundle = parse(raw)?;
    let items = analyze(db_path, app_data, &bundle)?;
    for item in &items {
        if !item.needs_store {
            continue;
        }
        let source = RecipeSource {
            url: item
                .app
                .source
                .as_ref()
                .map(|source| source.url.clone())
                .unwrap_or_else(|| "easyai.json".to_string()),
            sha256: catalog::sha256_hex(
                &serde_json::to_vec(&item.app.recipe)
                    .map_err(|err| format!("Failed to serialize '{}': {err}", item.entry.id))?,
            ),
            imported_at: catalog::imported_at(),
        };
        catalog::store_imported(app_data, &item.app.recipe, &source)?;
    }
    Ok(items.into_iter().map(|item| item.entry).collect())
}

struct Analyzed<'a> {
    app: &'a BundleApp,
    entry: BundleEntry,
    needs_store: bool,
}

fn parse(raw: &str) -> Result<Bundle, String> {
    if raw.len() > MAX_BUNDLE_BYTES {
        return Err("The easyai.json file is too large.".to_string());
    }
    let bundle: Bundle = serde_json::from_str(raw)
        .map_err(|err| format!("The easyai.json file is not valid: {err}"))?;
    if bundle.version != BUNDLE_VERSION {
        return Err(format!(
            "Unsupported easyai.json version {}. This build reads version {BUNDLE_VERSION}.",
            bundle.version
        ));
    }
    if bundle.apps.is_empty() {
        return Err("The easyai.json file contains no applications.".to_string());
    }

    let mut ids = HashSet::new();
    for (index, app) in bundle.apps.iter().enumerate() {
        app.recipe
            .validate()
            .map_err(|err| format!("Application {} in easyai.json: {err}", index + 1))?;
        if !ids.insert(app.recipe.id.clone()) {
            return Err(format!(
                "The easyai.json file lists '{}' more than once.",
                app.recipe.id
            ));
        }
    }
    Ok(bundle)
}

fn analyze<'a>(
    db_path: &Path,
    app_data: &Path,
    bundle: &'a Bundle,
) -> Result<Vec<Analyzed<'a>>, String> {
    let platform = probe::current_platform_id()?;
    let (os, arch) = platform
        .split_once('-')
        .ok_or_else(|| format!("Unsupported platform '{platform}'."))?;
    let existing = catalog::entries(app_data)?;
    let installed: HashMap<String, db::InstalledApp> = db::list_installed(db_path)
        .map_err(|err| format!("Failed to load installed applications: {err}"))?
        .into_iter()
        .map(|record| (record.app_id.clone(), record))
        .collect();

    let mut items = Vec::with_capacity(bundle.apps.len());
    for app in &bundle.apps {
        let recipe = &app.recipe;
        let known = existing
            .iter()
            .find(|entry| entry.recipe.id == recipe.id)
            .map(|entry| &entry.recipe);
        let active = known.unwrap_or(recipe);
        let record = installed.get(&recipe.id);
        let method_id = app
            .installed
            .as_ref()
            .and_then(|installed| installed.method_id.clone())
            .filter(|method| active.method(method).is_some());

        let (status, message, needs_store) = if let Some(record) = record {
            (
                "installed",
                Some(format!("Already installed (version {}).", record.version)),
                false,
            )
        } else if active.draft {
            (
                "unsupported",
                Some("Draft recipes have no install method.".to_string()),
                false,
            )
        } else if !active.supports_platform(os, arch) {
            (
                "unsupported",
                Some(format!("Does not support this machine ({platform}).")),
                false,
            )
        } else if known.is_some() {
            ("install", None, false)
        } else {
            match recipe.validate_for_import(&platform) {
                Ok(()) => ("install", None, true),
                Err(err) => ("unsupported", Some(err), false),
            }
        };

        items.push(Analyzed {
            app,
            entry: BundleEntry {
                id: recipe.id.clone(),
                name: recipe.name.clone(),
                version: record
                    .map(|record| record.version.clone())
                    .or_else(|| {
                        app.installed
                            .as_ref()
                            .map(|installed| installed.version.clone())
                    }),
                method_id,
                status: status.to_string(),
                message,
            },
            needs_store,
        });
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{export, import, parse, preview};
    use crate::catalog::{self, Recipe, RecipeSource};
    use crate::db::{self, InstalledApp};

    const RECIPE: &str = r#"{
        "id": "demo",
        "name": "Demo",
        "platforms": ["linux-x86_64"],
        "methods": [{
            "id": "release-linux",
            "kind": "release",
            "version": { "type": "github-release", "repo": "acme/demo" },
            "source": { "repo": "acme/demo", "asset": { "linux-x86_64": "demo.tar.gz" } },
            "uninstall": {}
        }]
    }"#;

    fn temp_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "easyai-bundle-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn rejects_an_unknown_bundle_version() {
        let err = parse(r#"{"version": 2, "apps": []}"#).unwrap_err();
        assert!(err.contains("Unsupported easyai.json version"), "{err}");
    }

    #[test]
    fn rejects_an_empty_bundle() {
        let err = parse(r#"{"version": 1, "apps": []}"#).unwrap_err();
        assert!(err.contains("contains no applications"), "{err}");
    }

    #[test]
    fn rejects_duplicate_application_ids() {
        let app = format!(r#"{{"recipe": {RECIPE}}}"#);
        let raw = format!(r#"{{"version": 1, "apps": [{app}, {app}]}}"#);
        let err = parse(&raw).unwrap_err();
        assert!(err.contains("more than once"), "{err}");
    }

    #[test]
    fn accepts_a_valid_bundle() {
        let app = format!(r#"{{"recipe": {RECIPE}}}"#);
        let raw = format!(r#"{{"version": 1, "apps": [{app}]}}"#);
        let bundle = parse(&raw).unwrap();
        assert_eq!(bundle.apps.len(), 1);
        assert_eq!(bundle.apps[0].recipe.id, "demo");
    }

    #[test]
    fn exports_installed_apps_and_reimports_them_elsewhere() {
        let source_data = temp_root("export-data");
        let source_db = source_data.join("easyai.db");
        db::init(&source_db).unwrap();
        let recipe: Recipe = serde_json::from_str(RECIPE).unwrap();
        catalog::store_imported(
            &source_data,
            &recipe,
            &RecipeSource {
                url: "https://github.com/acme/demo".to_string(),
                sha256: "source-sha".to_string(),
                imported_at: "1".to_string(),
            },
        )
        .unwrap();
        db::upsert_installed(
            &source_db,
            &InstalledApp {
                app_id: "demo".to_string(),
                version: "1.2.3".to_string(),
                install_dir: "/tmp/demo".to_string(),
                method: "release".to_string(),
                state: "stopped".to_string(),
                managed: true,
                method_id: Some("release-linux".to_string()),
                method_kind: Some("release".to_string()),
                provenance_json: None,
                bom_path: None,
            },
        )
        .unwrap();

        let exported = export(&source_db, &source_data).unwrap();
        let previewed = preview(&source_db, &source_data, &exported).unwrap();
        assert_eq!(previewed[0].status, "installed");
        assert_eq!(previewed[0].version.as_deref(), Some("1.2.3"));

        let target_data = temp_root("import-data");
        let target_db = target_data.join("easyai.db");
        db::init(&target_db).unwrap();
        let imported = import(&target_db, &target_data, &exported).unwrap();
        assert_eq!(imported[0].status, "install");
        assert_eq!(imported[0].method_id.as_deref(), Some("release-linux"));
        assert_eq!(
            catalog::find(&target_data, "demo").unwrap().recipe.id,
            "demo"
        );

        let _ = std::fs::remove_dir_all(source_data);
        let _ = std::fs::remove_dir_all(target_data);
    }
}
