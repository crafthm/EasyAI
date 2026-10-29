use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::catalog::{Method, MethodKind, Recipe};
use crate::probe::{ExistingInstall, HardwareProfile};

pub const TIERS: [&str; 5] = ["T0", "T1", "T2", "T3", "T4"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodSettings {
    pub order: Vec<String>,
    #[serde(default)]
    pub enabled: HashMap<String, bool>,
}

impl Default for MethodSettings {
    fn default() -> Self {
        Self {
            order: TIERS.iter().map(|tier| tier.to_string()).collect(),
            enabled: TIERS
                .iter()
                .map(|tier| (tier.to_string(), true))
                .collect(),
        }
    }
}

impl MethodSettings {
    pub fn tier_enabled(&self, tier: &str) -> bool {
        self.enabled.get(tier).copied().unwrap_or(true)
    }

    pub fn tier_score(&self, tier: &str) -> i32 {
        match self.order.iter().position(|item| item == tier) {
            Some(index) => ((self.order.len() - index) as i32) * 100,
            None => 0,
        }
    }
}

pub fn tier_for(method: &Method) -> &'static str {
    match method.kind {
        MethodKind::Portable => "T0",
        MethodKind::Release | MethodKind::Container => "T1",
        MethodKind::PackageManager => match method
            .package
            .as_ref()
            .map(|package| package.manager.as_str())
            .unwrap_or("")
        {
            "brew" | "winget" => "T2",
            _ => "T3",
        },
        MethodKind::Language => "T2",
        MethodKind::Script | MethodKind::Universal => "T3",
        MethodKind::Source | MethodKind::Vcs => "T4",
    }
}

#[derive(Debug, Clone, Default)]
pub struct Preferences {
    pub allow_elevation: bool,
    pub allow_advanced: bool,
    pub preferred_method: Option<String>,
    pub replace_existing: bool,
    pub settings: MethodSettings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rejection {
    pub method_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPlan {
    pub app_id: String,
    pub method_id: String,
    pub method_kind: String,
    pub tier: String,
    pub score: i32,
    pub rationale: Vec<String>,
    pub rejected: Vec<Rejection>,
    pub estimated_size: Option<u64>,
    pub needs_elevation: bool,
    pub advanced: bool,
    pub existing: Vec<ExistingInstall>,
    pub action: String,
}

pub fn resolve(
    recipe: &Recipe,
    profile: &HardwareProfile,
    preferences: &Preferences,
    existing: &[ExistingInstall],
) -> Result<InstallPlan, String> {
    let mut candidates: Vec<(&Method, i32, String)> = Vec::new();
    let mut rejected = Vec::new();

    for method in &recipe.methods {
        if let Err(reason) = hard_filter(method, profile, preferences) {
            rejected.push(Rejection {
                method_id: method.id.clone(),
                reason,
            });
            continue;
        }
        let score = score_method(method, &preferences.settings);
        let rationale = rationale(method, score);
        candidates.push((method, score, rationale));
    }

    candidates.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.id.cmp(&right.0.id))
    });
    let Some((method, score, rationale)) = candidates.first() else {
        return Err(format!(
            "No installation method is usable on this machine for '{}'.",
            recipe.id
        ));
    };
    for (other, other_score, _) in candidates.iter().skip(1) {
        rejected.push(Rejection {
            method_id: other.id.clone(),
            reason: format!("Lower score ({other_score}) than '{}'.", method.id),
        });
    }

    let matching_existing = existing
        .iter()
        .find(|item| item.method_id.as_deref() == Some(method.id.as_str()));
    let action = if matching_existing.is_some() && !preferences.replace_existing {
        "adopt"
    } else if !existing.is_empty() {
        "replace"
    } else {
        "install"
    };

    let mut rationale = vec![rationale.clone()];
    rationale.push(format!("Machine platform: {}.", profile.platform));
    if let Some(existing) = matching_existing {
        rationale.push(format!(
            "An existing installation was found through {} at {}.",
            existing.kind, existing.location
        ));
    }

    Ok(InstallPlan {
        app_id: recipe.id.clone(),
        method_id: method.id.clone(),
        method_kind: method.kind.as_str().to_string(),
        tier: tier_for(method).to_string(),
        score: *score,
        rationale,
        rejected,
        estimated_size: method.size,
        needs_elevation: method.elevation,
        advanced: method.advanced,
        existing: existing.to_vec(),
        action: action.to_string(),
    })
}

fn hard_filter(
    method: &Method,
    profile: &HardwareProfile,
    preferences: &Preferences,
) -> Result<(), String> {
    if !method.when.matches_platform(&profile.os, &profile.arch) {
        return Err(format!(
            "Requires platform {} {}, machine is {} {}.",
            list(&method.when.os),
            list(&method.when.arch),
            profile.os,
            profile.arch
        ));
    }
    if !method.when.matches_gpu(&profile.gpus) {
        return Err(format!(
            "Requires GPU {}, machine has {}.",
            method.when.gpu.join(", "),
            profile.gpus.join(", ")
        ));
    }
    if !method.when.matches_runtimes(&profile.runtimes) {
        return Err(format!(
            "Requires runtime {}, available: {}.",
            method.when.runtime.join(", "),
            sorted(&profile.runtimes)
        ));
    }
    if !method.when.matches_managers(&profile.managers) {
        return Err(format!(
            "Requires package manager {}, available: {}.",
            method.when.manager.join(", "),
            sorted(&profile.managers)
        ));
    }
    if !method.when.matches_toolchains(&profile.toolchains) {
        return Err(format!(
            "Requires toolchain {}, available: {}.",
            method.when.toolchain.join(", "),
            sorted(&profile.toolchains)
        ));
    }
    if let Some(required) = method.when.min_ram_mb {
        if profile.ram_mb < required {
            return Err(format!(
                "Requires {} MB RAM, machine has {} MB.",
                required, profile.ram_mb
            ));
        }
    }
    if let Some(required) = method.when.min_disk_mb {
        if profile.disk_mb < required {
            return Err(format!(
                "Requires {} MB free disk, machine has {} MB.",
                required, profile.disk_mb
            ));
        }
    }
    if method.elevation && !preferences.allow_elevation {
        return Err("Requires elevation, which is disabled by preference.".to_string());
    }
    if method.advanced && !preferences.allow_advanced {
        return Err("Advanced method is not enabled.".to_string());
    }
    let tier = tier_for(method);
    if !preferences.settings.tier_enabled(tier) {
        return Err(format!("Tier {tier} is disabled in settings."));
    }
    if let Some(preferred) = &preferences.preferred_method {
        if preferred != &method.id {
            return Err(format!("User selected method '{preferred}'."));
        }
    }
    Ok(())
}

fn score_method(method: &Method, settings: &MethodSettings) -> i32 {
    let tier = tier_for(method);
    let mut score = settings.tier_score(tier) + method.prefer;
    if method.elevation {
        score -= 20;
    }
    if method.advanced {
        score -= 15;
    }
    score
}

fn rationale(method: &Method, score: i32) -> String {
    let mut parts = vec![format!(
        "Selected method '{}' ({}, {}) with score {score}.",
        method.id,
        method.kind.as_str(),
        tier_for(method)
    )];
    if method.elevation {
        parts.push("Requires elevation.".to_string());
    } else {
        parts.push("User-space install.".to_string());
    }
    if method.advanced {
        parts.push("Advanced method; explicit opt-in is required.".to_string());
    }
    parts.join(" ")
}

fn list(values: &[String]) -> String {
    if values.is_empty() {
        "any".to_string()
    } else {
        values.join(", ")
    }
}

fn sorted(values: &std::collections::HashSet<String>) -> String {
    let mut values: Vec<&String> = values.iter().collect();
    values.sort();
    values
        .into_iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::{resolve, Preferences};
    use crate::catalog::Recipe;
    use crate::probe::HardwareProfile;

    fn profile() -> HardwareProfile {
        HardwareProfile {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            platform: "linux-x86_64".to_string(),
            gpus: vec!["cpu".to_string()],
            runtimes: HashSet::new(),
            managers: HashSet::new(),
            toolchains: HashSet::new(),
            ram_mb: 8192,
            disk_mb: 20_000,
        }
    }

    fn recipe(methods: serde_json::Value) -> Recipe {
        serde_json::from_value(json!({
            "id": "demo",
            "name": "Demo",
            "platforms": ["linux-x86_64"],
            "methods": methods
        }))
        .unwrap()
    }

    fn release(id: &str, prefer: i32) -> serde_json::Value {
        json!({
            "id": id,
            "kind": "release",
            "prefer": prefer,
            "source": { "repo": "owner/repo", "asset": { "linux-x86_64": "demo.tar.gz" } },
            "version": { "type": "github-release", "repo": "owner/repo" },
            "uninstall": { "remove_paths": ["{app_dir}"] }
        })
    }

    #[test]
    fn selects_the_highest_scoring_method() {
        let recipe = recipe(json!([release("low", 10), release("high", 90)]));
        let plan = resolve(&recipe, &profile(), &Preferences::default(), &[]).unwrap();
        assert_eq!(plan.method_id, "high");
        assert!(plan.score > 0);
    }

    #[test]
    fn tie_breaks_by_method_id() {
        let recipe = recipe(json!([release("b", 50), release("a", 50)]));
        let plan = resolve(&recipe, &profile(), &Preferences::default(), &[]).unwrap();
        assert_eq!(plan.method_id, "a");
    }

    fn release_with(id: &str, prefer: i32, when: serde_json::Value) -> serde_json::Value {
        let mut value = release(id, prefer);
        value["when"] = when;
        value
    }

    #[test]
    fn filters_cover_each_hard_constraint() {
        let recipe = recipe(json!([
            release("baseline", 10),
            release_with("mac", 90, json!({ "os": ["macos"] })),
            release_with("gpu", 90, json!({ "gpu": ["cuda"] })),
            release_with("tool", 90, json!({ "toolchain": ["rustc"] })),
            release_with("ram", 90, json!({ "min_ram_mb": 999999 })),
            release_with("disk", 90, json!({ "min_disk_mb": 999999 })),
            {
                "id": "elev",
                "kind": "package-manager",
                "elevation": true,
                "prefer": 90,
                "package": { "manager": "apt-get", "package": "demo" },
                "version": { "type": "registry", "package": "demo" },
                "uninstall": { "native": true }
            }
        ]));
        let plan = resolve(&recipe, &profile(), &Preferences::default(), &[]).unwrap();
        assert_eq!(plan.method_id, "baseline");
        let reasons: Vec<&str> = plan
            .rejected
            .iter()
            .map(|item| item.reason.as_str())
            .collect();
        assert!(reasons.iter().any(|reason| reason.contains("Requires platform")));
        assert!(reasons.iter().any(|reason| reason.contains("Requires GPU")));
        assert!(reasons.iter().any(|reason| reason.contains("Requires toolchain")));
        assert!(reasons.iter().any(|reason| reason.contains("MB RAM")));
        assert!(reasons.iter().any(|reason| reason.contains("free disk")));
        assert!(reasons.iter().any(|reason| reason.contains("Requires elevation")));
    }

    #[test]
    fn resolution_is_repeatable_and_serializable() {
        let recipe = recipe(json!([release("b", 50), release("a", 50)]));
        let first = resolve(&recipe, &profile(), &Preferences::default(), &[]).unwrap();
        let second = resolve(&recipe, &profile(), &Preferences::default(), &[]).unwrap();
        assert_eq!(first.method_id, second.method_id);
        assert_eq!(
            first
                .rejected
                .iter()
                .map(|item| item.method_id.clone())
                .collect::<Vec<_>>(),
            second
                .rejected
                .iter()
                .map(|item| item.method_id.clone())
                .collect::<Vec<_>>()
        );
        let value = serde_json::to_value(&first).unwrap();
        for key in [
            "appId",
            "methodId",
            "methodKind",
            "tier",
            "score",
            "rationale",
            "rejected",
            "estimatedSize",
            "needsElevation",
            "advanced",
            "existing",
            "action",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
    }

    #[test]
    fn hard_filters_report_reasons() {
        let recipe = recipe(json!([{
            "id": "container",
            "kind": "container",
            "when": { "runtime": ["docker"] },
            "image": { "image": "demo", "tag": "latest" },
            "version": { "type": "image", "image": "demo" },
            "uninstall": { "native": true }
        }, release("release", 10)]));
        let plan = resolve(&recipe, &profile(), &Preferences::default(), &[]).unwrap();
        assert_eq!(plan.method_id, "release");
        assert!(plan
            .rejected
            .iter()
            .any(|item| item.method_id == "container" && item.reason.contains("runtime")));
    }

    #[test]
    fn any_of_runtime_is_accepted() {
        let recipe = recipe(json!([{
            "id": "container",
            "kind": "container",
            "prefer": 90,
            "when": { "runtime": ["docker", "podman"] },
            "image": { "image": "demo", "tag": "latest" },
            "version": { "type": "image", "image": "demo" },
            "uninstall": { "native": true }
        }, release("release", 10)]));
        let mut profile = profile();
        profile.runtimes.insert("docker".to_string());
        let plan = resolve(&recipe, &profile, &Preferences::default(), &[]).unwrap();
        assert_eq!(plan.method_id, "container");
        assert!(plan
            .rejected
            .iter()
            .any(|item| item.method_id == "release" && item.reason.contains("Lower score")));
    }

    fn portable(id: &str, prefer: i32) -> serde_json::Value {
        json!({
            "id": id,
            "kind": "portable",
            "prefer": prefer,
            "source": { "url": "https://example.com/demo.zip" },
            "version": { "type": "github-release", "repo": "owner/repo" },
            "uninstall": { "keep_paths": [] }
        })
    }

    #[test]
    fn tier_priority_beats_method_score() {
        let recipe = recipe(json!([portable("portable", 0), release("release", 90)]));
        let plan = resolve(&recipe, &profile(), &Preferences::default(), &[]).unwrap();
        assert_eq!(plan.method_id, "portable");
        assert_eq!(plan.tier, "T0");
    }

    #[test]
    fn tier_order_is_user_configurable() {
        let recipe = recipe(json!([portable("portable", 0), release("release", 90)]));
        let settings = super::MethodSettings {
            order: vec![
                "T1".to_string(),
                "T0".to_string(),
                "T2".to_string(),
                "T3".to_string(),
                "T4".to_string(),
            ],
            enabled: std::collections::HashMap::new(),
        };
        let plan = resolve(
            &recipe,
            &profile(),
            &Preferences {
                settings,
                ..Preferences::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(plan.method_id, "release");
    }

    #[test]
    fn disabled_tier_is_rejected() {
        let recipe = recipe(json!([portable("portable", 0), release("release", 90)]));
        let mut settings = super::MethodSettings::default();
        settings.enabled.insert("T0".to_string(), false);
        let plan = resolve(
            &recipe,
            &profile(),
            &Preferences {
                settings,
                ..Preferences::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(plan.method_id, "release");
        assert!(plan
            .rejected
            .iter()
            .any(|item| item.method_id == "portable" && item.reason.contains("T0")));
    }

    #[test]
    fn advanced_methods_require_opt_in() {
        let recipe = recipe(json!([{
            "id": "source",
            "kind": "source",
            "advanced": true,
            "source": {
                "git": "https://example.com/repo",
                "ref": "v1.0.0",
                "build": { "commands": [["make"]] }
            },
            "version": { "type": "git" },
            "uninstall": { "remove_paths": ["{app_dir}"] }
        }]));
        let err = resolve(&recipe, &profile(), &Preferences::default(), &[]).unwrap_err();
        assert!(err.contains("No installation method"), "{err}");
        let plan = resolve(
            &recipe,
            &profile(),
            &Preferences {
                allow_advanced: true,
                ..Preferences::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(plan.method_id, "source");
        assert!(plan.advanced);
    }

    #[test]
    fn preferred_method_filters_others() {
        let recipe = recipe(json!([release("a", 90), release("b", 10)]));
        let plan = resolve(
            &recipe,
            &profile(),
            &Preferences {
                preferred_method: Some("b".to_string()),
                ..Preferences::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(plan.method_id, "b");
        assert!(plan.rejected.iter().any(|item| item.method_id == "a"));
    }

    #[test]
    fn existing_install_can_be_replaced() {
        let recipe = recipe(json!([release("release", 80)]));
        let existing = vec![crate::probe::ExistingInstall {
            method_id: Some("other".to_string()),
            kind: "package-manager".to_string(),
            location: "demo".to_string(),
            version: Some("0.9.0".to_string()),
            managed: false,
        }];
        let plan = resolve(&recipe, &profile(), &Preferences::default(), &existing).unwrap();
        assert_eq!(plan.action, "replace");
        let plan = resolve(
            &recipe,
            &profile(),
            &Preferences {
                replace_existing: true,
                ..Preferences::default()
            },
            &existing,
        )
        .unwrap();
        assert_eq!(plan.action, "replace");
    }

    #[test]
    fn existing_install_is_adopted() {
        let recipe = recipe(json!([release("release", 80)]));
        let existing = vec![crate::probe::ExistingInstall {
            method_id: Some("release".to_string()),
            kind: "release".to_string(),
            location: "/tmp/demo".to_string(),
            version: Some("1.0.0".to_string()),
            managed: true,
        }];
        let plan = resolve(&recipe, &profile(), &Preferences::default(), &existing).unwrap();
        assert_eq!(plan.action, "adopt");
    }
}