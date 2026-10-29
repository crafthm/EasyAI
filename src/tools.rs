use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub mutating: bool,
    pub inputs: &'static [&'static str],
}

pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "list_apps",
        description: "List catalog applications with their lifecycle state.",
        mutating: false,
        inputs: &[],
    },
    ToolSpec {
        name: "get_app_detail",
        description: "Read one application's version, install path, and health.",
        mutating: false,
        inputs: &["app_id"],
    },
    ToolSpec {
        name: "plan_install",
        description: "Resolve the installation method and show the plan without changing the machine.",
        mutating: false,
        inputs: &["app_id", "method_id", "allow_advanced", "allow_elevation"],
    },
    ToolSpec {
        name: "install_app",
        description: "Install an application using the selected method.",
        mutating: true,
        inputs: &[
            "app_id",
            "method_id",
            "allow_advanced",
            "allow_elevation",
            "origin",
            "confirmed",
        ],
    },
    ToolSpec {
        name: "start_app",
        description: "Start an installed application.",
        mutating: true,
        inputs: &["app_id", "origin", "confirmed"],
    },
    ToolSpec {
        name: "stop_app",
        description: "Stop a running application.",
        mutating: true,
        inputs: &["app_id", "origin", "confirmed"],
    },
    ToolSpec {
        name: "uninstall_app",
        description: "Uninstall an application while keeping user data.",
        mutating: true,
        inputs: &["app_id", "origin", "confirmed"],
    },
    ToolSpec {
        name: "list_jobs",
        description: "List lifecycle jobs for an application.",
        mutating: false,
        inputs: &["app_id"],
    },
    ToolSpec {
        name: "get_bom",
        description: "Read the bill of materials for an installed application.",
        mutating: false,
        inputs: &["app_id"],
    },
    ToolSpec {
        name: "get_method_settings",
        description: "Read the install-tier priority settings.",
        mutating: false,
        inputs: &[],
    },
    ToolSpec {
        name: "save_method_settings",
        description: "Change the install-tier priority settings.",
        mutating: true,
        inputs: &["settings", "origin", "confirmed"],
    },
    ToolSpec {
        name: "get_job_log",
        description: "Read the persisted log of a lifecycle job.",
        mutating: false,
        inputs: &["job_id"],
    },
    ToolSpec {
        name: "preview_recipe",
        description: "Fetch and validate a recipe without importing it.",
        mutating: false,
        inputs: &["url"],
    },
    ToolSpec {
        name: "import_recipe",
        description: "Add a reviewed recipe to the local catalog.",
        mutating: true,
        inputs: &["url", "sha256", "origin", "confirmed"],
    },
    ToolSpec {
        name: "remove_recipe",
        description: "Remove an imported recipe when its app is not installed.",
        mutating: true,
        inputs: &["app_id", "origin", "confirmed"],
    },
];

pub fn spec(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|tool| tool.name == name)
}

pub fn authorize(tool: &str, origin: &str, confirmed: bool) -> Result<(), String> {
    let spec = spec(tool).ok_or_else(|| format!("Unknown tool '{tool}'."))?;
    match origin {
        "ui" => Ok(()),
        "assistant" if spec.mutating && !confirmed => Err(format!(
            "Assistant-initiated '{tool}' requires explicit confirmation."
        )),
        "assistant" => Ok(()),
        other => Err(format!("Unsupported tool origin '{other}'.")),
    }
}

#[cfg(test)]
mod tests {
    use super::{authorize, spec, TOOLS};

    #[test]
    fn registry_covers_lifecycle_operations() {
        for name in [
            "install_app",
            "start_app",
            "stop_app",
            "uninstall_app",
            "plan_install",
            "get_app_detail",
            "get_job_log",
            "preview_recipe",
            "import_recipe",
            "remove_recipe",
        ] {
            assert!(spec(name).is_some(), "missing tool {name}");
        }
    }

    #[test]
    fn ui_calls_are_allowed() {
        assert!(authorize("install_app", "ui", false).is_ok());
    }

    #[test]
    fn unconfirmed_assistant_mutation_is_rejected() {
        let err = authorize("start_app", "assistant", false).unwrap_err();
        assert!(err.contains("confirmation"), "{err}");
    }

    #[test]
    fn confirmed_assistant_mutation_is_allowed() {
        assert!(authorize("start_app", "assistant", true).is_ok());
    }

    #[test]
    fn assistant_read_only_calls_are_allowed() {
        assert!(authorize("list_apps", "assistant", false).is_ok());
        assert!(authorize("get_job_log", "assistant", false).is_ok());
    }

    #[test]
    fn unknown_tool_is_rejected() {
        assert!(authorize("run_shell", "ui", true).is_err());
    }

    #[test]
    fn unknown_origin_is_rejected() {
        assert!(authorize("list_apps", "other", true).is_err());
    }

    #[test]
    fn registry_is_not_empty() {
        assert!(!TOOLS.is_empty());
    }
}