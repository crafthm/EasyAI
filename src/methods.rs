use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::catalog::{Method, MethodKind, Recipe, Service};
use crate::db::InstalledApp;
use crate::jobs::JobLogger;
use crate::probe;

pub struct MethodContext<'a> {
    pub recipe: &'a Recipe,
    pub method: &'a Method,
    pub app_data: &'a Path,
    pub platform: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Bom {
    pub app_id: String,
    pub method_id: String,
    pub paths: Vec<String>,
    pub services: Vec<String>,
    pub env: Vec<String>,
    pub commands: Vec<Vec<String>>,
}

pub fn install(ctx: &MethodContext<'_>, log: &mut JobLogger) -> Result<InstalledApp, String> {
    let app_dir = probe::app_dir(ctx.app_data, &ctx.recipe.id);
    let app = match ctx.method.kind {
        MethodKind::Release => install_release(ctx, log)?,
        MethodKind::Portable => install_portable(ctx, &app_dir, log)?,
        MethodKind::Container => install_container(ctx, log)?,
        MethodKind::PackageManager => install_package(ctx, log)?,
        MethodKind::Language => install_language(ctx, &app_dir, log)?,
        MethodKind::Script => install_script(ctx, &app_dir, log)?,
        MethodKind::Source => install_source(ctx, &app_dir, log)?,
        MethodKind::Vcs => install_vcs(ctx, &app_dir, log)?,
        MethodKind::Universal => install_universal(ctx, &app_dir, log)?,
    };
    let mut app = app;
    let bom = Bom {
        app_id: ctx.recipe.id.clone(),
        method_id: ctx.method.id.clone(),
        paths: vec![app.install_dir.clone()],
        services: Vec::new(),
        env: ctx
            .method
            .service
            .as_ref()
            .map(|service| {
                service
                    .env
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect()
            })
            .unwrap_or_default(),
        commands: ctx
            .method
            .all_commands()
            .into_iter()
            .cloned()
            .collect(),
    };
    let bom_path = write_bom(ctx.app_data, &bom)?;
    app.bom_path = Some(bom_path);
    Ok(app)
}

pub fn start(
    ctx: &MethodContext<'_>,
    record: &InstalledApp,
    log: &mut JobLogger,
) -> Result<(), String> {
    if ctx.method.kind == MethodKind::Container {
        let runtime = container_runtime(ctx.method)?;
        run_command(
            &[runtime.clone(), "start".to_string(), container_name(&ctx.recipe.id)],
            None,
            &HashMap::new(),
            log,
        )?;
        return wait_for_health(ctx.method, log);
    }
    let command = service_command(ctx, record)?;
    let logs = probe::logs_dir(ctx.app_data);
    fs::create_dir_all(&logs)
        .map_err(|err| format!("Failed to create log directory: {err}"))?;
    let service_log = logs.join(format!("{}-service.log", ctx.recipe.id));
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&service_log)
        .map_err(|err| format!("Failed to open service log: {err}"))?;
    let stderr = stdout
        .try_clone()
        .map_err(|err| format!("Failed to open service log: {err}"))?;

    let mut cmd = Command::new(&command[0]);
    cmd.args(&command[1..])
        .envs(service_env(ctx.method))
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd
        .spawn()
        .map_err(|err| format!("Failed to start {}: {err}", ctx.recipe.name))?;
    let pid = child.id() as i32;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    write_pid(ctx.app_data, &ctx.recipe.id, pid)?;
    log.log(format!("Started process {pid}."));
    wait_for_health(ctx.method, log)?;
    log.log("Health check passed.");
    Ok(())
}

pub fn stop(
    ctx: &MethodContext<'_>,
    _record: &InstalledApp,
    log: &mut JobLogger,
) -> Result<(), String> {
    if ctx.method.kind == MethodKind::Container {
        let runtime = container_runtime(ctx.method)?;
        run_command(
            &[runtime, "stop".to_string(), container_name(&ctx.recipe.id)],
            None,
            &HashMap::new(),
            log,
        )?;
        log.log("Container stopped.");
        return Ok(());
    }
    if let Some(pid) = read_pid(ctx.app_data, &ctx.recipe.id) {
        if process_alive(pid) {
            log.log(format!("Stopping process {pid}."));
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline && process_alive(pid) {
                std::thread::sleep(Duration::from_millis(100));
            }
            if process_alive(pid) {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
        }
        let _ = fs::remove_file(probe::pid_path(ctx.app_data, &ctx.recipe.id));
    }
    log.log("Stopped.");
    Ok(())
}

pub fn uninstall(
    ctx: &MethodContext<'_>,
    record: &InstalledApp,
    log: &mut JobLogger,
) -> Result<(), String> {
    let uninstall = ctx
        .method
        .uninstall
        .as_ref()
        .ok_or_else(|| "Method has no uninstall behavior.".to_string())?;

    if let Some(command) = &uninstall.command {
        let command = expand_command(command, record, &probe::app_dir(ctx.app_data, &ctx.recipe.id));
        run_command(&command, None, &HashMap::new(), log)?;
    }

    match ctx.method.kind {
        MethodKind::Container => {
            let runtime = container_runtime(ctx.method)?;
            let name = container_name(&ctx.recipe.id);
            let _ = run_command(
                &[runtime, "rm".to_string(), "-f".to_string(), name],
                None,
                &HashMap::new(),
                log,
            );
        }
        MethodKind::PackageManager => {
            if let Some(package) = &ctx.method.package {
                let command = manager_uninstall(&package.manager, &package.package, ctx.method.elevation);
                run_command(&command, None, &HashMap::new(), log)?;
            }
        }
        MethodKind::Universal => {
            if let Some(universal) = &ctx.method.universal {
                if universal.format == "flatpak" {
                    run_command(
                        &[
                            "flatpak".to_string(),
                            "uninstall".to_string(),
                            "-y".to_string(),
                            universal.reference.clone(),
                        ],
                        None,
                        &HashMap::new(),
                        log,
                    )?;
                } else if universal.format == "snap" {
                    run_command(
                        &[
                            "snap".to_string(),
                            "remove".to_string(),
                            universal.reference.clone(),
                        ],
                        None,
                        &HashMap::new(),
                        log,
                    )?;
                } else if universal.format == "nix" {
                    run_command(
                        &[
                            "nix".to_string(),
                            "profile".to_string(),
                            "remove".to_string(),
                            universal.reference.clone(),
                        ],
                        None,
                        &HashMap::new(),
                        log,
                    )?;
                }
            }
        }
        _ => {}
    }

    for path in uninstall
        .remove_paths
        .iter()
        .map(|path| expand_path(path, record, &probe::app_dir(ctx.app_data, &ctx.recipe.id)))
    {
        if path.exists() {
            if path.is_dir() {
                fs::remove_dir_all(&path)
                    .map_err(|err| format!("Failed to remove {}: {err}", path.display()))?;
            } else {
                fs::remove_file(&path)
                    .map_err(|err| format!("Failed to remove {}: {err}", path.display()))?;
            }
        }
    }
    let app_dir = probe::app_dir(ctx.app_data, &ctx.recipe.id);
    if !matches!(
        ctx.method.kind,
        MethodKind::PackageManager | MethodKind::Container | MethodKind::Universal
    ) && app_dir.exists()
    {
        fs::remove_dir_all(&app_dir)
            .map_err(|err| format!("Failed to remove {}: {err}", app_dir.display()))?;
    }
    let _ = fs::remove_file(probe::pid_path(ctx.app_data, &ctx.recipe.id));
    let _ = fs::remove_file(probe::bom_path(ctx.app_data, &ctx.recipe.id));
    log.log("Removed installed files.");
    Ok(())
}

pub fn health(method: &Method) -> bool {
    method
        .service
        .as_ref()
        .map(|service| health_service(service))
        .unwrap_or(false)
}

pub fn is_running(method: &Method, record: &InstalledApp, app_data: &Path) -> bool {
    if method.kind == MethodKind::Container {
        let Ok(runtime) = container_runtime(method) else {
            return false;
        };
        let output = Command::new(runtime)
            .args([
                "inspect",
                "-f",
                "{{.State.Running}}",
                &container_name(&record.app_id),
            ])
            .output();
        return output
            .map(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).trim() == "true"
            })
            .unwrap_or(false);
    }
    read_pid(app_data, &record.app_id)
        .map(process_alive)
        .unwrap_or(false)
}

fn install_release(
    ctx: &MethodContext<'_>,
    log: &mut JobLogger,
) -> Result<InstalledApp, String> {
    let source = ctx
        .method
        .source
        .as_ref()
        .ok_or_else(|| "Release method has no source.".to_string())?;
    let repo = source
        .repo
        .as_ref()
        .ok_or_else(|| "Release method has no repository.".to_string())?;
    let release = fetch_latest_release(repo)?;
    let version = normalize_version(&release.tag)?;
    let asset_name = probe::asset_for(source, ctx.platform)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| format!("Release {version} does not include '{asset_name}'."))?;

    let downloads = probe::downloads_dir(ctx.app_data);
    fs::create_dir_all(&downloads)
        .map_err(|err| format!("Failed to create download directory: {err}"))?;
    let archive = downloads.join(format!("{version}-{asset_name}"));
    ensure_archive(&asset.url, &archive, source.sha256.as_deref(), log)?;

    let versions = probe::versions_dir(ctx.app_data, &ctx.recipe.id);
    fs::create_dir_all(&versions)
        .map_err(|err| format!("Failed to create install directory: {err}"))?;
    let staging = versions.join(format!(".staging-{version}"));
    let install_dir = probe::version_dir(ctx.app_data, &ctx.recipe.id, &version);
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    extract(&archive, &staging)?;
    if install_dir.exists() {
        fs::remove_dir_all(&install_dir)
            .map_err(|err| format!("Failed to replace existing install: {err}"))?;
    }
    fs::rename(&staging, &install_dir)
        .map_err(|err| format!("Failed to activate the new version: {err}"))?;
    let _ = fs::remove_file(&archive);

    let app = installed(ctx, version.clone(), install_dir.to_string_lossy().to_string());
    verify_binary(ctx, &app, log)?;
    log.log(format!("Installed {} {version}.", ctx.recipe.name));
    Ok(app)
}

fn install_portable(
    ctx: &MethodContext<'_>,
    app_dir: &Path,
    log: &mut JobLogger,
) -> Result<InstalledApp, String> {
    let source = ctx
        .method
        .source
        .as_ref()
        .ok_or_else(|| "Portable method has no source.".to_string())?;
    let url = if let Some(url) = &source.url {
        url.clone()
    } else {
        let repo = source
            .repo
            .as_ref()
            .ok_or_else(|| "Portable method has no repository.".to_string())?;
        let release = fetch_latest_release(repo)?;
        let asset_name = probe::asset_for(source, ctx.platform)?;
        release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| format!("Release does not include '{asset_name}'."))?
            .url
            .clone()
    };
    let downloads = probe::downloads_dir(ctx.app_data);
    fs::create_dir_all(&downloads)
        .map_err(|err| format!("Failed to create download directory: {err}"))?;
    let extension = url
        .rsplit('.')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("zip");
    let archive = downloads.join(format!("{}-portable.{extension}", ctx.recipe.id));
    ensure_archive(&url, &archive, source.sha256.as_deref(), log)?;

    if app_dir.exists() {
        fs::remove_dir_all(app_dir)
            .map_err(|err| format!("Failed to replace the portable bundle: {err}"))?;
    }
    fs::create_dir_all(app_dir)
        .map_err(|err| format!("Failed to create the app directory: {err}"))?;
    extract(&archive, app_dir)?;
    let _ = fs::remove_file(&archive);

    let app = installed(
        ctx,
        "portable".to_string(),
        app_dir.to_string_lossy().to_string(),
    );
    log.log(format!("Installed {} portable bundle.", ctx.recipe.name));
    Ok(app)
}

fn install_container(
    ctx: &MethodContext<'_>,
    log: &mut JobLogger,
) -> Result<InstalledApp, String> {
    let image = ctx
        .method
        .image
        .as_ref()
        .ok_or_else(|| "Container method has no image.".to_string())?;
    let runtime = container_runtime(ctx.method)?;
    let image_ref = container_image(image)?;
    run_command(
        &[runtime.clone(), "pull".to_string(), image_ref.clone()],
        None,
        &HashMap::new(),
        log,
    )?;
    let name = container_name(&ctx.recipe.id);
    let _ = run_command(
        &[runtime.clone(), "rm".to_string(), "-f".to_string(), name.clone()],
        None,
        &HashMap::new(),
        log,
    );
    let command = container_run_command(ctx.method, &runtime, &name, &image_ref);
    run_command(&command, None, &HashMap::new(), log)?;
    let app = installed(ctx, image_ref.clone(), name.clone());
    wait_for_health(ctx.method, log)?;
    log.log(format!("Started container {name}."));
    Ok(app)
}

fn install_package(
    ctx: &MethodContext<'_>,
    log: &mut JobLogger,
) -> Result<InstalledApp, String> {
    let package = ctx
        .method
        .package
        .as_ref()
        .ok_or_else(|| "Package method has no package section.".to_string())?;
    let command = manager_install(
        &package.manager,
        &package.package,
        package.version.as_deref(),
        ctx.method.elevation,
    );
    run_command(&command, None, &HashMap::new(), log)?;
    let version = manager_version(&package.manager, &package.package)
        .unwrap_or_else(|| package.version.clone().unwrap_or_else(|| "unknown".to_string()));
    log.log(format!("Installed {} through {}.", package.package, package.manager));
    Ok(installed(ctx, version, package.package.clone()))
}

fn install_language(
    ctx: &MethodContext<'_>,
    app_dir: &Path,
    log: &mut JobLogger,
) -> Result<InstalledApp, String> {
    let language = ctx
        .method
        .language
        .as_ref()
        .ok_or_else(|| "Language method has no language section.".to_string())?;
    fs::create_dir_all(app_dir)
        .map_err(|err| format!("Failed to create the app directory: {err}"))?;
    let env_dir = app_dir.join(".venv");
    match language.ecosystem.as_str() {
        "python" => {
            let manager = language.manager.as_deref().unwrap_or("pip");
            if manager == "uv" && probe::find_in_path("uv").is_some() {
                run_command(
                    &[
                        "uv".to_string(),
                        "venv".to_string(),
                        env_dir.to_string_lossy().to_string(),
                    ],
                    None,
                    &HashMap::new(),
                    log,
                )?;
            } else {
                run_command(
                    &[
                        "python3".to_string(),
                        "-m".to_string(),
                        "venv".to_string(),
                        env_dir.to_string_lossy().to_string(),
                    ],
                    None,
                    &HashMap::new(),
                    log,
                )?;
            }
            let mut command = if manager == "uv" && probe::find_in_path("uv").is_some() {
                vec![
                    "uv".to_string(),
                    "pip".to_string(),
                    "install".to_string(),
                    "--python".to_string(),
                    env_dir.join("bin").join("python").to_string_lossy().to_string(),
                ]
            } else {
                vec![
                    env_dir.join("bin").join("pip").to_string_lossy().to_string(),
                    "install".to_string(),
                ]
            };
            if let Some(index) = &language.index {
                command.push("--index-url".to_string());
                command.push(index.clone());
            }
            if let Some(index) = &language.extra_index {
                command.push("--extra-index-url".to_string());
                command.push(index.clone());
            }
            command.push(match &language.version {
                Some(version) => format!("{}=={}", language.package, version),
                None => language.package.clone(),
            });
            run_command(&command, None, &HashMap::new(), log)?;
        }
        "npm" => {
            let mut command = vec![
                "npm".to_string(),
                "install".to_string(),
                "--prefix".to_string(),
                app_dir.to_string_lossy().to_string(),
            ];
            command.push(match &language.version {
                Some(version) => format!("{}@{}", language.package, version),
                None => language.package.clone(),
            });
            run_command(&command, None, &HashMap::new(), log)?;
        }
        "cargo" => {
            let mut command = vec![
                "cargo".to_string(),
                "install".to_string(),
                "--root".to_string(),
                app_dir.to_string_lossy().to_string(),
            ];
            if let Some(version) = &language.version {
                command.push("--version".to_string());
                command.push(version.clone());
            }
            command.push(language.package.clone());
            run_command(&command, None, &HashMap::new(), log)?;
        }
        "go" => {
            let version = language.version.clone().unwrap_or_else(|| "latest".to_string());
            let command = vec![
                "go".to_string(),
                "install".to_string(),
                format!("{}@{}", language.package, version),
            ];
            let mut env = HashMap::new();
            env.insert(
                "GOBIN".to_string(),
                app_dir.join("bin").to_string_lossy().to_string(),
            );
            run_command(&command, None, &env, log)?;
        }
        other => return Err(format!("Unsupported language ecosystem '{other}'.")),
    }
    let version = language
        .version
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    log.log(format!("Installed {} {}.", language.package, version));
    Ok(installed(ctx, version, app_dir.to_string_lossy().to_string()))
}

fn install_script(
    ctx: &MethodContext<'_>,
    app_dir: &Path,
    log: &mut JobLogger,
) -> Result<InstalledApp, String> {
    let script = ctx
        .method
        .script
        .as_ref()
        .ok_or_else(|| "Script method has no script section.".to_string())?;
    fs::create_dir_all(app_dir)
        .map_err(|err| format!("Failed to create the app directory: {err}"))?;
    let downloads = probe::downloads_dir(ctx.app_data);
    fs::create_dir_all(&downloads)
        .map_err(|err| format!("Failed to create download directory: {err}"))?;
    let path = downloads.join(format!("{}-install.sh", ctx.recipe.id));
    ensure_archive(&script.url, &path, script.sha256.as_deref(), log)?;
    run_command(
        &[
            "sh".to_string(),
            path.to_string_lossy().to_string(),
        ],
        Some(app_dir),
        &service_env(ctx.method),
        log,
    )?;
    let version = if let Some(probe) = &script.probe {
        run_command(probe, Some(app_dir), &HashMap::new(), log)
            .ok()
            .and_then(|output| output.lines().next().map(|line| line.trim().to_string()))
            .unwrap_or_else(|| "unknown".to_string())
    } else {
        "unknown".to_string()
    };
    log.log(format!("Install script finished ({version})."));
    Ok(installed(ctx, version, app_dir.to_string_lossy().to_string()))
}

fn install_source(
    ctx: &MethodContext<'_>,
    app_dir: &Path,
    log: &mut JobLogger,
) -> Result<InstalledApp, String> {
    let source = ctx
        .method
        .source
        .as_ref()
        .ok_or_else(|| "Source method has no source.".to_string())?;
    let git = source
        .git
        .as_ref()
        .ok_or_else(|| "Source method has no git URL.".to_string())?;
    let reference = source
        .git_ref
        .as_ref()
        .ok_or_else(|| "Source method has no pinned ref.".to_string())?;
    let build = source
        .build
        .as_ref()
        .ok_or_else(|| "Source method has no build section.".to_string())?;
    if app_dir.exists() {
        fs::remove_dir_all(app_dir)
            .map_err(|err| format!("Failed to clear the source directory: {err}"))?;
    }
    fs::create_dir_all(app_dir)
        .map_err(|err| format!("Failed to create the source directory: {err}"))?;
    let checkout = app_dir.join("src");
    let cloned = run_command(
        &[
            "git".to_string(),
            "clone".to_string(),
            "--depth".to_string(),
            "1".to_string(),
            "--branch".to_string(),
            reference.clone(),
            git.clone(),
            checkout.to_string_lossy().to_string(),
        ],
        None,
        &HashMap::new(),
        log,
    );
    if cloned.is_err() {
        let _ = fs::remove_dir_all(&checkout);
        run_command(
            &[
                "git".to_string(),
                "clone".to_string(),
                git.clone(),
                checkout.to_string_lossy().to_string(),
            ],
            None,
            &HashMap::new(),
            log,
        )?;
        run_command(
            &[
                "git".to_string(),
                "-C".to_string(),
                checkout.to_string_lossy().to_string(),
                "checkout".to_string(),
                reference.clone(),
            ],
            None,
            &HashMap::new(),
            log,
        )?;
    }
    for command in &build.commands {
        let command = expand_paths_in_command(command, app_dir, &checkout);
        run_command(&command, Some(&checkout), &HashMap::new(), log)?;
    }
    log.log(format!("Built source at {reference}."));
    Ok(installed(
        ctx,
        reference.clone(),
        app_dir.to_string_lossy().to_string(),
    ))
}

fn install_vcs(
    ctx: &MethodContext<'_>,
    app_dir: &Path,
    log: &mut JobLogger,
) -> Result<InstalledApp, String> {
    let source = ctx
        .method
        .source
        .as_ref()
        .ok_or_else(|| "VCS method has no source.".to_string())?;
    let git = source
        .git
        .as_ref()
        .ok_or_else(|| "VCS method has no git URL.".to_string())?;
    let reference = source
        .git_ref
        .as_ref()
        .ok_or_else(|| "VCS method has no pinned ref.".to_string())?;
    fs::create_dir_all(app_dir)
        .map_err(|err| format!("Failed to create the app directory: {err}"))?;
    if let Some(language) = &ctx.method.language {
        match language.ecosystem.as_str() {
            "python" => {
                let env = app_dir.join(".venv");
                run_command(
                    &[
                        "python3".to_string(),
                        "-m".to_string(),
                        "venv".to_string(),
                        env.to_string_lossy().to_string(),
                    ],
                    None,
                    &HashMap::new(),
                    log,
                )?;
                run_command(
                    &[
                        env.join("bin").join("pip").to_string_lossy().to_string(),
                        "install".to_string(),
                        format!("{} @ git+{}@{}", language.package, git, reference),
                    ],
                    None,
                    &HashMap::new(),
                    log,
                )?;
            }
            "cargo" => {
                run_command(
                    &[
                        "cargo".to_string(),
                        "install".to_string(),
                        "--root".to_string(),
                        app_dir.to_string_lossy().to_string(),
                        "--git".to_string(),
                        git.clone(),
                        "--tag".to_string(),
                        reference.clone(),
                        language.package.clone(),
                    ],
                    None,
                    &HashMap::new(),
                    log,
                )?;
            }
            "npm" => {
                run_command(
                    &[
                        "npm".to_string(),
                        "install".to_string(),
                        "--prefix".to_string(),
                        app_dir.to_string_lossy().to_string(),
                        format!("git+{}#{}", git, reference),
                    ],
                    None,
                    &HashMap::new(),
                    log,
                )?;
            }
            other => return Err(format!("Unsupported VCS ecosystem '{other}'.")),
        }
    } else if let Some(package) = &ctx.method.package {
        run_command(
            &[
                "cargo".to_string(),
                "install".to_string(),
                "--root".to_string(),
                app_dir.to_string_lossy().to_string(),
                "--git".to_string(),
                git.clone(),
                "--tag".to_string(),
                reference.clone(),
                package.package.clone(),
            ],
            None,
            &HashMap::new(),
            log,
        )?;
    }
    log.log(format!("Installed from git ref {reference}."));
    Ok(installed(
        ctx,
        reference.clone(),
        app_dir.to_string_lossy().to_string(),
    ))
}

fn install_universal(
    ctx: &MethodContext<'_>,
    app_dir: &Path,
    log: &mut JobLogger,
) -> Result<InstalledApp, String> {
    let universal = ctx
        .method
        .universal
        .as_ref()
        .ok_or_else(|| "Universal method has no universal section.".to_string())?;
    fs::create_dir_all(app_dir)
        .map_err(|err| format!("Failed to create the app directory: {err}"))?;
    match universal.format.as_str() {
        "appimage" => {
            let path = app_dir.join("app.AppImage");
            ensure_archive(&universal.reference, &path, None, log)?;
            let mut permissions = fs::metadata(&path)
                .map_err(|err| format!("Failed to read AppImage metadata: {err}"))?
                .permissions();
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions)
                .map_err(|err| format!("Failed to make AppImage executable: {err}"))?;
        }
        "flatpak" => {
            run_command(
                &[
                    "flatpak".to_string(),
                    "install".to_string(),
                    "-y".to_string(),
                    "--noninteractive".to_string(),
                    universal.reference.clone(),
                ],
                None,
                &HashMap::new(),
                log,
            )?;
        }
        "snap" => {
            run_command(
                &[
                    "snap".to_string(),
                    "install".to_string(),
                    universal.reference.clone(),
                ],
                None,
                &HashMap::new(),
                log,
            )?;
        }
        "nix" => {
            run_command(
                &[
                    "nix".to_string(),
                    "profile".to_string(),
                    "install".to_string(),
                    universal.reference.clone(),
                ],
                None,
                &HashMap::new(),
                log,
            )?;
        }
        other => return Err(format!("Unsupported universal format '{other}'.")),
    }
    log.log(format!("Installed {} via {}.", ctx.recipe.name, universal.format));
    Ok(installed(
        ctx,
        universal.reference.clone(),
        app_dir.to_string_lossy().to_string(),
    ))
}

fn installed(ctx: &MethodContext<'_>, version: String, install_dir: String) -> InstalledApp {
    let provenance = serde_json::json!({
        "methodId": ctx.method.id,
        "kind": ctx.method.kind.as_str(),
        "platform": ctx.platform,
        "source": ctx.method.source,
        "image": ctx.method.image,
        "package": ctx.method.package,
        "language": ctx.method.language,
        "script": ctx.method.script,
        "universal": ctx.method.universal,
    });
    InstalledApp {
        app_id: ctx.recipe.id.clone(),
        version,
        install_dir,
        method: ctx.method.kind.as_str().to_string(),
        state: "stopped".to_string(),
        managed: true,
        method_id: Some(ctx.method.id.clone()),
        method_kind: Some(ctx.method.kind.as_str().to_string()),
        provenance_json: Some(provenance.to_string()),
        bom_path: None,
    }
}

fn verify_binary(ctx: &MethodContext<'_>, app: &InstalledApp, log: &mut JobLogger) -> Result<(), String> {
    let command = service_command(ctx, app)?;
    if command.is_empty() {
        return Ok(());
    }
    let output = run_command(
        &[command[0].clone(), "--version".to_string()],
        None,
        &HashMap::new(),
        log,
    )?;
    log.log(format!("Version check: {}", output.trim()));
    Ok(())
}

fn service_command(ctx: &MethodContext<'_>, record: &InstalledApp) -> Result<Vec<String>, String> {
    let service = ctx
        .method
        .service
        .as_ref()
        .ok_or_else(|| format!("Method '{}' has no service command.", ctx.method.id))?;
    if service.command.is_empty() {
        return Err(format!("Method '{}' has no start command.", ctx.method.id));
    }
    Ok(expand_command(
        &service.command,
        record,
        &probe::app_dir(ctx.app_data, &ctx.recipe.id),
    ))
}

fn expand_command(command: &[String], record: &InstalledApp, app_dir: &Path) -> Vec<String> {
    command
        .iter()
        .map(|part| {
            part.replace("{install_dir}", &record.install_dir)
                .replace("{app_dir}", &app_dir.to_string_lossy())
                .replace("{env}", &app_dir.join(".venv").to_string_lossy())
        })
        .collect()
}

fn expand_paths_in_command(command: &[String], app_dir: &Path, checkout: &Path) -> Vec<String> {
    command
        .iter()
        .map(|part| {
            part.replace("{app_dir}", &app_dir.to_string_lossy())
                .replace("{source_dir}", &checkout.to_string_lossy())
        })
        .collect()
}

fn expand_path(path: &str, record: &InstalledApp, app_dir: &Path) -> PathBuf {
    PathBuf::from(
        path.replace("{install_dir}", &record.install_dir)
            .replace("{app_dir}", &app_dir.to_string_lossy()),
    )
}

fn service_env(method: &Method) -> HashMap<String, String> {
    method
        .service
        .as_ref()
        .map(|service| service.env.clone())
        .unwrap_or_default()
}

fn wait_for_health(method: &Method, log: &mut JobLogger) -> Result<(), String> {
    let Some(service) = &method.service else {
        return Ok(());
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if health_service(service) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    log.log("Health check timed out.");
    Err(format!(
        "{} did not become healthy within 30 seconds.",
        service.health_url
    ))
}

fn health_service(service: &Service) -> bool {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
    else {
        return false;
    };
    client
        .get(&service.health_url)
        .send()
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

fn write_pid(app_data: &Path, app_id: &str, pid: i32) -> Result<(), String> {
    let path = probe::pid_path(app_data, app_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create run directory: {err}"))?;
    }
    fs::write(&path, format!("{pid}\n"))
        .map_err(|err| format!("Failed to record process id: {err}"))
}

fn read_pid(app_data: &Path, app_id: &str) -> Option<i32> {
    fs::read_to_string(probe::pid_path(app_data, app_id))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn process_alive(pid: i32) -> bool {
    pid > 0 && unsafe { libc::kill(pid, 0) == 0 }
}

pub fn read_bom(app_data: &Path, app_id: &str) -> Result<Bom, String> {
    let path = probe::bom_path(app_data, app_id);
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read the BOM at {}: {err}", path.display()))?;
    serde_json::from_str(&content).map_err(|err| format!("Failed to parse the BOM: {err}"))
}

fn write_bom(app_data: &Path, bom: &Bom) -> Result<String, String> {
    let path = probe::bom_path(app_data, &bom.app_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create the BOM directory: {err}"))?;
    }
    let content = serde_json::to_vec_pretty(bom)
        .map_err(|err| format!("Failed to serialize the BOM: {err}"))?;
    fs::write(&path, content).map_err(|err| format!("Failed to write the BOM: {err}"))?;
    Ok(path.to_string_lossy().to_string())
}

fn container_runtime(method: &Method) -> Result<String, String> {
    let requested = method
        .image
        .as_ref()
        .and_then(|image| image.runtime.as_deref())
        .unwrap_or("auto");
    if requested != "auto" {
        return Ok(requested.to_string());
    }
    for runtime in ["docker", "podman"] {
        if probe::find_in_path(runtime).is_some() {
            return Ok(runtime.to_string());
        }
    }
    Err("No container runtime (docker or podman) is available.".to_string())
}

fn container_name(app_id: &str) -> String {
    format!("easyai-{app_id}")
}

fn container_image(image: &crate::catalog::ImageSpec) -> Result<String, String> {
    if let Some(digest) = &image.digest {
        Ok(format!("{}@{}", image.image, digest))
    } else if let Some(tag) = &image.tag {
        Ok(format!("{}:{}", image.image, tag))
    } else {
        Err("Container image has no tag or digest.".to_string())
    }
}

fn container_run_command(
    method: &Method,
    runtime: &str,
    name: &str,
    image: &str,
) -> Vec<String> {
    let image_spec = method.image.as_ref().expect("validated container method");
    let mut command = vec![
        runtime.to_string(),
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        name.to_string(),
    ];
    if let Some(restart) = &image_spec.restart {
        command.push("--restart".to_string());
        command.push(restart.clone());
    }
    for port in &image_spec.ports {
        command.push("-p".to_string());
        command.push(port.clone());
    }
    for volume in &image_spec.volumes {
        command.push("-v".to_string());
        command.push(volume.clone());
    }
    for (key, value) in &image_spec.env {
        command.push("-e".to_string());
        command.push(format!("{key}={value}"));
    }
    if image_spec.gpu.as_deref() == Some("auto") || image_spec.gpu.as_deref() == Some("cuda") {
        if runtime == "podman" {
            command.push("--device".to_string());
            command.push("nvidia.com/gpu=all".to_string());
        } else {
            command.push("--gpus".to_string());
            command.push("all".to_string());
        }
    }
    command.push(image.to_string());
    command
}

fn manager_install(
    manager: &str,
    package: &str,
    version: Option<&str>,
    elevation: bool,
) -> Vec<String> {
    let mut command = match manager {
        "brew" => vec!["brew".to_string(), "install".to_string(), package.to_string()],
        "winget" => vec![
            "winget".to_string(),
            "install".to_string(),
            "--id".to_string(),
            package.to_string(),
            "--exact".to_string(),
            "--silent".to_string(),
        ],
        "apt" | "apt-get" => vec!["apt-get".to_string(), "install".to_string(), "-y".to_string(), package.to_string()],
        "dnf" | "rpm" => vec!["dnf".to_string(), "install".to_string(), "-y".to_string(), package.to_string()],
        "pacman" => vec!["pacman".to_string(), "-S".to_string(), "--noconfirm".to_string(), package.to_string()],
        _ => vec![manager.to_string(), "install".to_string(), package.to_string()],
    };
    if let Some(version) = version {
        if manager == "brew" && command.len() >= 3 {
            command[2] = format!("{package}@{version}");
        }
    }
    if elevation && unsafe { libc::geteuid() } != 0 {
        command.insert(0, "sudo".to_string());
        command.insert(1, "-n".to_string());
    }
    command
}

fn manager_uninstall(manager: &str, package: &str, elevation: bool) -> Vec<String> {
    let mut command = match manager {
        "brew" => vec!["brew".to_string(), "uninstall".to_string(), package.to_string()],
        "winget" => vec![
            "winget".to_string(),
            "uninstall".to_string(),
            "--id".to_string(),
            package.to_string(),
            "--exact".to_string(),
            "--silent".to_string(),
        ],
        "apt" | "apt-get" => vec!["apt-get".to_string(), "remove".to_string(), "-y".to_string(), package.to_string()],
        "dnf" | "rpm" => vec!["dnf".to_string(), "remove".to_string(), "-y".to_string(), package.to_string()],
        "pacman" => vec!["pacman".to_string(), "-Rns".to_string(), "--noconfirm".to_string(), package.to_string()],
        _ => vec![manager.to_string(), "uninstall".to_string(), package.to_string()],
    };
    if elevation && unsafe { libc::geteuid() } != 0 {
        command.insert(0, "sudo".to_string());
        command.insert(1, "-n".to_string());
    }
    command
}

fn manager_version(manager: &str, package: &str) -> Option<String> {
    let output = match manager {
        "brew" => Command::new("brew").args(["list", "--versions", package]).output().ok()?,
        "winget" => Command::new("winget").args(["list", "--id", package, "--exact"]).output().ok()?,
        "apt" | "apt-get" => Command::new("dpkg-query").args(["-W", "-f=${Version}", package]).output().ok()?,
        "dnf" | "rpm" => Command::new("rpm").args(["-q", "--qf", "%{VERSION}", package]).output().ok()?,
        "pacman" => Command::new("pacman").args(["-Q", package]).output().ok()?,
        _ => return None,
    };
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return None;
    }
    match manager {
        "brew" => text.split_whitespace().nth(1).map(str::to_string).or(Some(text)),
        "pacman" => text.split_whitespace().nth(1).map(str::to_string),
        _ => Some(text),
    }
}

fn run_command(
    args: &[String],
    cwd: Option<&Path>,
    env: &HashMap<String, String>,
    log: &mut JobLogger,
) -> Result<String, String> {
    if args.is_empty() {
        return Err("Empty command.".to_string());
    }
    log.log(format!("$ {}", args.join(" ")));
    let mut command = Command::new(&args[0]);
    command.args(&args[1..]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    if !env.is_empty() {
        command.envs(env);
    }
    let output = command
        .output()
        .map_err(|err| format!("Failed to run '{}': {err}", args[0]))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    for line in stdout.lines().chain(stderr.lines()) {
        log.log(line);
    }
    if !output.status.success() {
        return Err(format!(
            "Command failed ({}): {}",
            output.status,
            args.join(" ")
        ));
    }
    Ok(stdout)
}

fn ensure_archive(
    url: &str,
    path: &Path,
    expected_sha256: Option<&str>,
    log: &mut JobLogger,
) -> Result<(), String> {
    if path.exists() {
        if let Some(expected) = expected_sha256 {
            if file_sha256(path)? == expected {
                log.log("Using the previously downloaded archive.");
                return Ok(());
            }
        }
    }
    log.log(format!("Downloading {url}"));
    let client = reqwest::blocking::Client::builder()
        .user_agent("EasyAI/0.1")
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|err| format!("Failed to create HTTP client: {err}"))?;
    let mut response = client
        .get(url)
        .send()
        .map_err(|err| format!("Download failed: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("Download failed: server returned {}.", response.status()));
    }
    let mut file = File::create(path)
        .map_err(|err| format!("Failed to create {}: {err}", path.display()))?;
    io::copy(&mut response, &mut file)
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))?;
    if let Some(expected) = expected_sha256 {
        let actual = file_sha256(path)?;
        if actual != expected {
            let _ = fs::remove_file(path);
            return Err("Downloaded artifact checksum does not match the recipe.".to_string());
        }
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("Failed to open {}: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn extract(archive: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|err| format!("Failed to create extraction directory: {err}"))?;
    let name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if name.ends_with(".zip") {
        let file = File::open(archive)
            .map_err(|err| format!("Failed to open archive: {err}"))?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|err| format!("Failed to read zip archive: {err}"))?;
        for index in 0..zip.len() {
            let mut entry = zip
                .by_index(index)
                .map_err(|err| format!("Failed to read zip entry: {err}"))?;
            let Some(out) = entry.enclosed_name().map(|name| destination.join(name)) else {
                continue;
            };
            if entry.is_dir() {
                fs::create_dir_all(&out)
                    .map_err(|err| format!("Failed to create directory: {err}"))?;
            } else {
                if let Some(parent) = out.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|err| format!("Failed to create directory: {err}"))?;
                }
                let mut output = File::create(&out)
                    .map_err(|err| format!("Failed to create {}: {err}", out.display()))?;
                io::copy(&mut entry, &mut output)
                    .map_err(|err| format!("Failed to extract {}: {err}", out.display()))?;
            }
        }
        return Ok(());
    }
    if name.ends_with(".7z") {
        sevenz_rust::decompress_file(archive, destination)
            .map_err(|err| format!("Failed to extract 7z archive: {err}"))?;
        return Ok(());
    }
    let file = File::open(archive)
        .map_err(|err| format!("Failed to open archive: {err}"))?;
    if name.ends_with(".zst") {
        let decoder = zstd::stream::read::Decoder::new(file)
            .map_err(|err| format!("Failed to read zstd archive: {err}"))?;
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(destination)
            .map_err(|err| format!("Failed to extract archive: {err}"))
    } else if name.ends_with(".tar") {
        let mut archive = tar::Archive::new(file);
        archive
            .unpack(destination)
            .map_err(|err| format!("Failed to extract archive: {err}"))
    } else {
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(destination)
            .map_err(|err| format!("Failed to extract archive: {err}"))
    }
}

#[derive(Debug)]
struct Release {
    tag: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug)]
struct ReleaseAsset {
    name: String,
    url: String,
}

fn fetch_latest_release(repo: &str) -> Result<Release, String> {
    #[derive(serde::Deserialize)]
    struct ApiRelease {
        tag_name: String,
        assets: Vec<ApiAsset>,
    }
    #[derive(serde::Deserialize)]
    struct ApiAsset {
        name: String,
        browser_download_url: String,
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent("EasyAI/0.1")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| format!("Failed to create HTTP client: {err}"))?;
    let response = client
        .get(format!("https://api.github.com/repos/{repo}/releases/latest"))
        .send()
        .map_err(|err| format!("Could not reach GitHub: {err}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "GitHub release lookup failed for {repo} ({}).",
            response.status()
        ));
    }
    let release: ApiRelease = response
        .json()
        .map_err(|err| format!("Invalid release metadata: {err}"))?;
    Ok(Release {
        tag: release.tag_name,
        assets: release
            .assets
            .into_iter()
            .map(|asset| ReleaseAsset {
                name: asset.name,
                url: asset.browser_download_url,
            })
            .collect(),
    })
}

fn normalize_version(tag: &str) -> Result<String, String> {
    let version = tag.trim().trim_start_matches(['v', 'V']);
    if version.is_empty()
        || !version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        return Err(format!("The release tag '{tag}' is not a usable version."));
    }
    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        container_image, container_run_command, expand_command, manager_install,
        manager_uninstall, read_bom, write_bom, Bom,
    };
    use crate::catalog::{ImageSpec, Method, MethodKind};
    use serde_json::json;

    fn container_method() -> Method {
        serde_json::from_value(json!({
            "id": "container",
            "kind": "container",
            "image": {
                "image": "ollama/ollama",
                "tag": "latest",
                "runtime": "docker",
                "volumes": ["ollama:/root/.ollama"],
                "ports": ["11434:11434"],
                "env": {"OLLAMA_HOST": "127.0.0.1:11434"},
                "gpu": "auto",
                "restart": "unless-stopped"
            },
            "version": {"type": "image", "image": "ollama/ollama"},
            "uninstall": {"native": true}
        }))
        .unwrap()
    }

    #[test]
    fn builds_container_run_command() {
        let method = container_method();
        assert_eq!(method.kind, MethodKind::Container);
        let command = container_run_command(
            &method,
            "docker",
            "easyai-ollama",
            "ollama/ollama:latest",
        );
        let joined = command.join(" ");
        assert!(joined.contains("docker run -d --name easyai-ollama"));
        assert!(joined.contains("--restart unless-stopped"));
        assert!(joined.contains("-p 11434:11434"));
        assert!(joined.contains("-v ollama:/root/.ollama"));
        assert!(joined.contains("--gpus all"));
        assert!(joined.ends_with("ollama/ollama:latest"));
    }

    #[test]
    fn expands_placeholders() {
        let record = crate::db::InstalledApp {
            app_id: "demo".to_string(),
            version: "1".to_string(),
            install_dir: "/opt/demo".to_string(),
            method: "release".to_string(),
            state: "stopped".to_string(),
            managed: true,
            method_id: None,
            method_kind: None,
            provenance_json: None,
            bom_path: None,
        };
        assert_eq!(
            expand_command(
                &["{install_dir}/bin/demo".to_string(), "{app_dir}/x".to_string()],
                &record,
                std::path::Path::new("/data/demo")
            ),
            vec!["/opt/demo/bin/demo", "/data/demo/x"]
        );
    }

    #[test]
    fn builds_package_manager_commands() {
        assert_eq!(
            manager_install("brew", "ollama", None, false),
            vec!["brew", "install", "ollama"]
        );
        assert_eq!(
            manager_uninstall("winget", "Ollama.Ollama", false),
            vec![
                "winget",
                "uninstall",
                "--id",
                "Ollama.Ollama",
                "--exact",
                "--silent"
            ]
        );
        assert_eq!(
            manager_install("apt", "demo", None, false),
            vec!["apt-get", "install", "-y", "demo"]
        );
        assert_eq!(
            manager_uninstall("pacman", "demo", false),
            vec!["pacman", "-Rns", "--noconfirm", "demo"]
        );
    }

    #[test]
    fn pins_container_digest() {
        let spec: ImageSpec = serde_json::from_value(json!({
            "image": "demo/demo",
            "digest": "sha256:abc"
        }))
        .unwrap();
        assert_eq!(
            container_image(&spec).unwrap(),
            "demo/demo@sha256:abc"
        );
    }

    #[test]
    fn extracts_zip_and_tar_gz_archives() {
        let directory = std::env::temp_dir().join(format!(
            "easyai-extract-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();

        let zip_path = directory.join("bundle.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("hello.txt", options).unwrap();
            use std::io::Write;
            writer.write_all(b"hello").unwrap();
            writer.finish().unwrap();
        }
        let zip_out = directory.join("zip-out");
        super::extract(&zip_path, &zip_out).unwrap();
        assert_eq!(
            std::fs::read_to_string(zip_out.join("hello.txt")).unwrap(),
            "hello"
        );

        let tar_path = directory.join("bundle.tar.gz");
        {
            let file = std::fs::File::create(&tar_path).unwrap();
            let encoder =
                flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(5);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "hello.txt", &b"hello"[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let tar_out = directory.join("tar-out");
        super::extract(&tar_path, &tar_out).unwrap();
        assert_eq!(
            std::fs::read_to_string(tar_out.join("hello.txt")).unwrap(),
            "hello"
        );

        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn writes_a_bill_of_materials() {
        let directory = std::env::temp_dir().join(format!(
            "easyai-bom-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let bom = Bom {
            app_id: "demo".to_string(),
            method_id: "release".to_string(),
            paths: vec!["/tmp/demo".to_string()],
            services: vec!["demo.service".to_string()],
            env: vec!["DEMO=1".to_string()],
            commands: vec![vec!["demo".to_string(), "serve".to_string()]],
        };
        write_bom(&directory, &bom).unwrap();
        let parsed = read_bom(&directory, "demo").unwrap();
        assert_eq!(parsed.app_id, "demo");
        assert_eq!(parsed.method_id, "release");
        assert_eq!(parsed.paths, vec!["/tmp/demo"]);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}