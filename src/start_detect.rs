//! Generic, application-agnostic start-command discovery.
//!
//! An executable in an install directory is a *candidate*, not a service.
//! This module derives candidate start commands from a program's own `--help`
//! output using generic vocabulary (server/daemon/serve/run/start, foreground
//! flags), and verifies them in a bounded sandbox before anything is stored.

use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::catalog::{DetachMode, HealthCheck};
use crate::methods::{
    self, classify_early_exit, launch_executables, log_excerpt, EarlyExitKind,
};

const HELP_TIMEOUT: Duration = Duration::from_secs(2);
const PROBE_DURATION: Duration = Duration::from_millis(1500);
const MAX_CANDIDATES: usize = 8;

/// Words that usually introduce a long-running subcommand. Deliberately
/// generic: nothing here knows about a particular application.
const LONG_RUNNING: &[&str] = &[
    "serve", "server", "start", "run", "up", "daemon", "listen", "watch", "agent",
];
const SECONDARY: &[&str] = &[
    "start", "stop", "run", "status", "restart", "up", "serve", "foreground",
];
const FOREGROUND_FLAGS: &[&str] = &[
    "--foreground",
    "--no-daemon",
    "--no-detach",
    "--no-background",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartCandidate {
    /// argv with `{install_dir}` left in place for the recipe/override.
    pub command: Vec<String>,
    pub health: HealthCheck,
    pub detach: DetachMode,
    pub confidence: String,
    pub reason: String,
    /// Needed for self-daemonizing commands, where killing the PID group
    /// cannot reach the detached worker.
    pub stop_command: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartVerification {
    pub ok: bool,
    pub status: Option<i32>,
    pub output: String,
    pub reason: String,
}

fn is_command_word(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= 24
        && word
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        && word.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic())
}

fn push_unique(chains: &mut Vec<Vec<String>>, chain: Vec<String>) {
    if !chains.contains(&chain) {
        chains.push(chain);
    }
}

/// Derives subcommand chains from help text. Kept separate from filesystem
/// probing so it can be tested against arbitrary CLI help output.
fn parse_help_chains(help: &str) -> Vec<Vec<String>> {
    let lower = help.to_ascii_lowercase();
    let words: Vec<String> = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_')
        .filter(|word| is_command_word(word))
        .map(str::to_string)
        .collect();
    let mut chains = Vec::new();
    for word in &words {
        if LONG_RUNNING.contains(&word.as_str()) {
            push_unique(&mut chains, vec![word.clone()]);
        }
    }
    for pair in words.windows(2) {
        if LONG_RUNNING.contains(&pair[0].as_str())
            && SECONDARY.contains(&pair[1].as_str())
            && pair[0] != pair[1]
        {
            push_unique(&mut chains, vec![pair[0].clone(), pair[1].clone()]);
        }
    }
    chains
}

fn help_foreground_flag(help: &str) -> Option<&'static str> {
    let lower = help.to_ascii_lowercase();
    FOREGROUND_FLAGS
        .iter()
        .copied()
        .find(|flag| lower.contains(flag))
}

/// Probes `<binary> --help` in its own process group with a hard timeout.
fn capture_help(binary: &Path, args: &[String]) -> Option<String> {
    let mut cmd = Command::new(binary);
    cmd.args(args)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().ok()?;
    let pid = child.id() as i32;
    let deadline = Instant::now() + HELP_TIMEOUT;
    let finished = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if Instant::now() >= deadline => break false,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break false,
        }
    };
    if !finished {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        let _ = child.wait();
        return None;
    }
    let output = child.wait_with_output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Some(text.chars().take(64 * 1024).collect())
}

/// Suggests start commands for an installed application. Every suggestion is
/// untrusted until `verify_command` confirms the process stays alive.
pub fn detect_candidates(install_dir: &Path, app_name: &str) -> Vec<StartCandidate> {
    let mut result: Vec<StartCandidate> = Vec::new();
    for binary in launch_executables(install_dir, app_name)
        .into_iter()
        .take(3)
    {
        let relative = binary
            .strip_prefix(install_dir)
            .unwrap_or(&binary)
            .to_string_lossy()
            .to_string();
        let placeholder = format!("{{install_dir}}/{relative}");
        let Some(top_help) = capture_help(&binary, &[]) else {
            continue;
        };
        let top_flag = help_foreground_flag(&top_help);
        let mut chains = parse_help_chains(&top_help);
        // Flags such as `--foreground` usually live in a subcommand's help
        // rather than the top-level help, so expand one level generically.
        for chain in chains.clone() {
            let Some(sub_help) = capture_help(&binary, &chain) else {
                continue;
            };
            for word in parse_help_chains(&sub_help) {
                if word.len() == 1 && !chain.contains(&word[0]) {
                    let mut extended = chain.clone();
                    extended.extend(word);
                    push_unique(&mut chains, extended);
                }
            }
        }
        chains.truncate(12);

        for chain in chains {
            let sub_help = capture_help(&binary, &chain).unwrap_or_default();
            let flag = top_flag.or_else(|| help_foreground_flag(&sub_help));
            let help_text = format!("{}{}", top_help.to_ascii_lowercase(), sub_help.to_ascii_lowercase());
            let has_status = help_text.contains("status");
            let has_stop = help_text.contains("stop");

            // A foreground variant keeps the process attached to EasyAI's
            // process group, so it is always preferred over a daemonizing one.
            if let Some(flag) = flag {
                let mut command = vec![placeholder.clone()];
                command.extend(chain.clone());
                command.push(flag.to_string());
                result.push(StartCandidate {
                    command,
                    health: HealthCheck::Process,
                    detach: DetachMode::Foreground,
                    confidence: "high".to_string(),
                    reason: format!(
                        "help lists `{}` and `{}` keeps it in the foreground",
                        chain.join(" "),
                        flag
                    ),
                    stop_command: None,
                });
            }
            let mut command = vec![placeholder.clone()];
            command.extend(chain.clone());
            // Only trust a daemonizing chain when its own help also advertises
            // status and stop, so both health tracking and Stop are possible.
            let daemonizing = chain.len() == 2 && chain[1] == "start" && has_status && has_stop;
            let health = if daemonizing {
                let mut status = vec![placeholder.clone(), chain[0].clone()];
                status.push("status".to_string());
                HealthCheck::Exec { command: status }
            } else {
                HealthCheck::Process
            };
            let stop_command = daemonizing.then(|| {
                vec![placeholder.clone(), chain[0].clone(), "stop".to_string()]
            });
            result.push(StartCandidate {
                command,
                health,
                detach: if daemonizing {
                    DetachMode::Daemonize
                } else {
                    DetachMode::Foreground
                },
                confidence: if flag.is_some() {
                    "medium".to_string()
                } else {
                    "low".to_string()
                },
                reason: format!("help lists `{}`", chain.join(" ")),
                stop_command,
            });
        }
        // A GUI app's own binary may itself be the long-running command. It is
        // kept as a low-confidence candidate that still requires verification.
        result.push(StartCandidate {
            command: vec![placeholder],
            health: HealthCheck::Process,
            detach: DetachMode::Foreground,
            confidence: "low".to_string(),
            reason: "the executable itself; verify it stays running".to_string(),
            stop_command: None,
        });
    }
    // Prefer verified-looking foreground variants over daemonizing ones, then
    // keep the list bounded so the UI stays usable.
    result.sort_by_key(|candidate| match candidate.confidence.as_str() {
        "high" => 0,
        "medium" => 1,
        _ => 2,
    });
    result.truncate(MAX_CANDIDATES);
    result
}

fn expand(command: &[String], install_dir: &str, app_dir: &Path) -> Vec<String> {
    command
        .iter()
        .map(|part| {
            part.replace("{install_dir}", install_dir)
                .replace("{app_dir}", &app_dir.to_string_lossy())
                .replace("{env}", &app_dir.join(".venv").to_string_lossy())
        })
        .collect()
}

fn probe_log_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("easyai-start-probe-{}-{stamp}.log", std::process::id()))
}

fn read_probe_output(path: &Path) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let mut buffer = Vec::new();
    let _ = file.read_to_end(&mut buffer);
    let text = String::from_utf8_lossy(&buffer);
    text.chars().take(1000).collect()
}

fn verification_reason(status: ExitStatus, text: &str) -> String {
    match classify_early_exit(status, text) {
        EarlyExitKind::OneShotCli => "The command printed CLI help and exited, so it is a one-shot tool rather than a long-running service.".to_string(),
        EarlyExitKind::AuthRequired => "The command exited because authentication or first-time setup is required.".to_string(),
        EarlyExitKind::Crashed => "The command exited with a failure; check its output and environment.".to_string(),
        EarlyExitKind::Unknown => "The command exited before it could be considered running.".to_string(),
    }
}

/// Runs a candidate in a bounded sandbox and reports whether it stayed alive.
/// The whole process group is killed afterwards, so a probe never leaves a
/// service behind.
pub fn verify_command(
    command: &[String],
    install_dir: &str,
    app_dir: &Path,
) -> Result<StartVerification, String> {
    let expanded = expand(command, install_dir, app_dir);
    if expanded.is_empty() {
        return Err("The start command is empty.".to_string());
    }
    let log_path = probe_log_path();
    let output = File::create(&log_path)
        .map_err(|err| format!("Failed to create the start probe log: {err}"))?;
    let stderr = output
        .try_clone()
        .map_err(|err| format!("Failed to open the start probe log: {err}"))?;

    let mut cmd = Command::new(&expanded[0]);
    cmd.args(&expanded[1..])
        .stdin(Stdio::null())
        .stdout(output)
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
        .map_err(|err| format!("Failed to run '{}': {err}", expanded[0]))?;
    let pid = child.id() as i32;
    let exited = methods::exited_during_grace(&mut child, PROBE_DURATION)
        .map_err(|err| format!("Failed to probe the start command: {err}"))?;
    if exited.is_none() {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.wait();

    let text = read_probe_output(&log_path);
    let excerpt = log_excerpt(&log_path);
    let _ = fs::remove_file(&log_path);

    Ok(match exited {
        None => StartVerification {
            ok: true,
            status: None,
            output: excerpt,
            reason: format!(
                "Stayed running for {} ms; the process group was stopped after the probe.",
                PROBE_DURATION.as_millis()
            ),
        },
        Some(status) => StartVerification {
            ok: false,
            status: status.code(),
            output: excerpt,
            reason: verification_reason(status, &text),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{parse_help_chains, verify_command, FOREGROUND_FLAGS};

    #[test]
    fn parses_generic_daemon_chains_from_help() {
        let help = "\
Work with things from the command line.

USAGE
  tool <command> <subcommand> [flags]

RUNTIME COMMANDS
  daemon:   Control the local daemon
  status:   Show status

EXAMPLES
  $ tool daemon start
  $ tool serve --port 8080

FLAGS
      --foreground   Run in the foreground
";
        let chains = parse_help_chains(help);
        assert!(chains.contains(&vec!["daemon".to_string(), "start".to_string()]));
        assert!(chains.contains(&vec!["serve".to_string()]));
        assert!(FOREGROUND_FLAGS.contains(&"--foreground"));
    }

    #[test]
    fn detection_finds_foreground_flags_in_subcommand_help() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "easyai-start-detect-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("tool");
        fs::write(
            &binary,
            b"#!/bin/sh\n\
if [ \"$1\" = daemon ] && [ \"$2\" = start ] && [ \"$3\" = --help ]; then\n\
  echo FLAGS; echo '  --foreground  Run in the foreground'\n\
elif [ \"$1\" = daemon ] && [ \"$2\" = --help ]; then\n\
  echo COMMANDS; echo '  start: Start the daemon'\n\
else\n\
  echo USAGE; echo '  tool <command>'; echo COMMANDS; echo '  daemon: Control the daemon'\n\
fi\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).unwrap();

        let candidates = super::detect_candidates(&dir, "tool");
        assert!(
            candidates.iter().any(|candidate| candidate.command
                == vec![
                    "{install_dir}/tool".to_string(),
                    "daemon".to_string(),
                    "start".to_string(),
                    "--foreground".to_string(),
                ]),
            "candidates: {candidates:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn verification_rejects_a_one_shot_help_cli() {
        let dir = std::env::temp_dir().join(format!(
            "easyai-probe-cli-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("tool");
        fs::write(&binary, "#!/bin/sh\necho USAGE\necho '  tool <command> --help'\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).unwrap();

        let result = verify_command(&vec![binary.to_string_lossy().to_string()], "/tmp", &dir)
            .unwrap();
        assert!(!result.ok);
        assert!(result.reason.contains("one-shot"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn verification_accepts_a_long_running_command() {
        let result = verify_command(
            &vec!["sleep".to_string(), "5".to_string()],
            "/tmp",
            &std::env::temp_dir(),
        )
        .unwrap();
        assert!(result.ok);
        assert!(result.status.is_none());
    }
}
