use std::path::{Path, PathBuf};

use crate::catalog::Recipe;

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

pub fn asset_for(recipe: &Recipe, platform: &str) -> Result<String, String> {
    if !recipe.platforms.iter().any(|supported| supported == platform) {
        return Err(format!(
            "{} does not support this platform ({platform}).",
            recipe.name
        ));
    }
    let install = recipe
        .install
        .as_ref()
        .ok_or_else(|| format!("{} has no install method.", recipe.name))?;
    install
        .asset
        .get(platform)
        .cloned()
        .ok_or_else(|| format!("{} has no release asset for {platform}.", recipe.name))
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{asset_for, platform_id_for};
    use crate::catalog::Recipe;

    fn recipe() -> Recipe {
        serde_json::from_value(json!({
            "id": "demo",
            "name": "Demo",
            "platforms": ["linux-x86_64", "linux-aarch64"],
            "install": {
                "kind": "github-release-binary",
                "repo": "owner/repo",
                "asset": {
                    "linux-x86_64": "demo-amd64.tgz",
                    "linux-aarch64": "demo-arm64.tgz"
                }
            },
            "service": { "command": ["demo"], "health_url": "http://127.0.0.1:9999" },
            "uninstall": {}
        }))
        .unwrap()
    }

    #[test]
    fn maps_supported_platforms() {
        assert_eq!(platform_id_for("linux", "x86_64").unwrap(), "linux-x86_64");
        assert_eq!(
            platform_id_for("linux", "aarch64").unwrap(),
            "linux-aarch64"
        );
        assert_eq!(platform_id_for("macos", "aarch64").unwrap(), "macos-aarch64");
    }

    #[test]
    fn rejects_unsupported_platforms() {
        assert!(platform_id_for("freebsd", "x86_64").is_err());
        assert!(platform_id_for("linux", "riscv64").is_err());
    }

    #[test]
    fn selects_asset_for_platform() {
        assert_eq!(
            asset_for(&recipe(), "linux-x86_64").unwrap(),
            "demo-amd64.tgz"
        );
        assert_eq!(
            asset_for(&recipe(), "linux-aarch64").unwrap(),
            "demo-arm64.tgz"
        );
    }

    #[test]
    fn rejects_unsupported_asset_platform() {
        let err = asset_for(&recipe(), "macos-aarch64").unwrap_err();
        assert!(err.contains("does not support"), "{err}");
    }
}