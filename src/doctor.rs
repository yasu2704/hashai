//! Bounded, redaction-safe readiness diagnostics for the hashai CLI.

use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use serde::Serialize;

use crate::{
    config::{Config, Shell},
    integration::IntegrationManager,
    runner::{CodexRunner, RunRequest},
};

pub const SCHEMA_VERSION: u8 = 1;
const IDS: &[&str] = &[
    "platform",
    "shell.kind",
    "shell.version",
    "codex.command",
    "codex.version",
    "codex.exec",
    "codex.ephemeral",
    "codex.ignore_user_config",
    "codex.ignore_rules",
    "codex.model",
    "codex.config",
    "codex.sandbox",
    "codex.disable.shell_tool",
    "codex.disable.browser_use",
    "codex.disable.computer_use",
    "codex.disable.apps",
    "codex.skip_git_repo_check",
    "codex.output_schema",
    "codex.output_last_message",
    "codex.project_doc_max_bytes",
    "codex.project_doc_fallback_filenames",
    "auth",
    "model_reasoning",
    "keybinding",
    "integration",
    "json_processing",
    "outside_git_repository",
    "live_probe",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    Pass,
    Warn,
    Fail,
}
impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub status: Status,
    pub message: &'static str,
    pub exit: i32,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u8,
    pub mode: &'static str,
    pub checks: Vec<Check>,
    pub overall: Status,
    pub exit: i32,
}
enum Probe {
    Ok(String),
    Missing,
    Failed,
    Cancelled,
}

pub fn run(
    config: &Config,
    requested_shell: Option<&str>,
    live: bool,
    os: &str,
    cancelled: &AtomicBool,
) -> Report {
    let mut checks = Vec::with_capacity(IDS.len());
    let platform_ok = matches!(os, "linux" | "macos");
    let parsed_shell = requested_shell
        .map(Shell::parse)
        .unwrap_or_else(|| Ok(config.shell.clone()))
        .and_then(|shell| shell.resolve(std::env::var("SHELL").ok().as_deref()));
    let shell_kind_ok = matches!(parsed_shell, Ok(Shell::Bash | Shell::Zsh | Shell::Fish));
    checks.push(named(
        "platform",
        if platform_ok {
            pass("supported operating system")
        } else {
            fail(9, "unsupported operating system")
        },
    ));
    checks.push(named(
        "shell.kind",
        if shell_kind_ok {
            pass("supported shell")
        } else {
            fail(9, "unsupported shell")
        },
    ));
    let shell = parsed_shell.unwrap_or(Shell::Bash);
    checks.push(named(
        "shell.version",
        match if shell_kind_ok {
            shell_version(&shell, cancelled)
        } else {
            Probe::Failed
        } {
            Probe::Ok(text) if version_at_least(&text, &shell) => pass("supported shell version"),
            Probe::Ok(_) => fail(9, "unsupported shell version"),
            Probe::Missing | Probe::Failed => fail(1, "shell version unavailable or invalid"),
            Probe::Cancelled => fail(7, "shell version inspection cancelled"),
        },
    ));

    let codex = codex_executable();
    let version = bounded(&codex, &["--version"], cancelled);
    let present = matches!(version, Probe::Ok(_));
    let version_exit = if matches!(version, Probe::Missing) {
        3
    } else {
        1
    };
    checks.push(named(
        "codex.command",
        match version {
            Probe::Ok(_) => pass("Codex CLI is available"),
            Probe::Missing => fail(3, "Codex CLI is missing"),
            Probe::Cancelled => fail(7, "Codex CLI inspection cancelled"),
            Probe::Failed => fail(1, "Codex CLI could not be inspected"),
        },
    ));
    checks.push(named(
        "codex.version",
        if present {
            pass("Codex version is readable")
        } else {
            fail(version_exit, "Codex version is unavailable")
        },
    ));
    let help = if present {
        bounded(&codex, &["exec", "--help"], cancelled)
    } else {
        Probe::Missing
    };
    let help_text = match &help {
        Probe::Ok(text) => text.as_str(),
        _ => "",
    };
    for (id, needle, live_only) in capabilities() {
        checks.push(named(
            id,
            if live_only {
                warn("exact value is verified only by --live")
            } else if present && has_token(help_text, needle) {
                pass("required Codex capability available")
            } else {
                fail(1, "required Codex capability missing")
            },
        ));
    }
    checks.push(named("auth", static_auth(&codex, cancelled)));
    checks.push(named(
        "model_reasoning",
        warn("model and reasoning availability requires --live"),
    ));
    checks.push(named(
        "keybinding",
        if live {
            keymap_probe(&shell, config, cancelled)
        } else {
            warn("keybinding inspection requires --live")
        },
    ));
    checks.push(named("integration", integration_check(&shell)));
    checks.push(named(
        "json_processing",
        pass("JSON processing is built into hashai"),
    ));
    checks.push(named(
        "outside_git_repository",
        pass("doctor uses no Git repository state"),
    ));
    let live_result = if live {
        live_probe(config, codex, cancelled, &mut checks)
    } else {
        warn("live probe was not requested")
    };
    checks.push(named("live_probe", live_result));
    debug_assert_eq!(checks.iter().map(|check| check.id).collect::<Vec<_>>(), IDS);
    finalize(if live { "live" } else { "static" }, checks)
}

fn capabilities() -> [(&'static str, &'static str, bool); 16] {
    [
        ("codex.exec", "exec", false),
        ("codex.ephemeral", "--ephemeral", false),
        ("codex.ignore_user_config", "--ignore-user-config", false),
        ("codex.ignore_rules", "--ignore-rules", false),
        ("codex.model", "--model", false),
        ("codex.config", "--config", false),
        ("codex.sandbox", "--sandbox", false),
        ("codex.disable.shell_tool", "--disable", true),
        ("codex.disable.browser_use", "--disable", true),
        ("codex.disable.computer_use", "--disable", true),
        ("codex.disable.apps", "--disable", true),
        ("codex.skip_git_repo_check", "--skip-git-repo-check", false),
        ("codex.output_schema", "--output-schema", false),
        ("codex.output_last_message", "--output-last-message", false),
        ("codex.project_doc_max_bytes", "--config", true),
        ("codex.project_doc_fallback_filenames", "--config", true),
    ]
}

fn static_auth(codex: &PathBuf, cancelled: &AtomicBool) -> Check {
    let Probe::Ok(help) = bounded(codex, &["login", "--help"], cancelled) else {
        return warn("authentication status is not advertised");
    };
    if !help.contains("status") {
        return warn("authentication status is not advertised");
    }
    match bounded(codex, &["login", "status"], cancelled) {
        Probe::Ok(text)
            if contains_any(&text, &["logged in", "authenticated"])
                && !contains_any(&text, &["not logged", "unauthenticated"]) =>
        {
            pass("authentication is available")
        }
        Probe::Ok(text)
            if contains_any(&text, &["not logged", "unauthenticated", "logged out"]) =>
        {
            fail(4, "Codex CLI is not authenticated")
        }
        Probe::Cancelled => fail(7, "authentication status inspection cancelled"),
        _ => warn("authentication status is unknown"),
    }
}

fn live_probe(
    config: &Config,
    codex: PathBuf,
    cancelled: &AtomicBool,
    checks: &mut [Check],
) -> Check {
    match CodexRunner::new().run(
        RunRequest {
            executable: codex,
            prompt: "Return a harmless shell command in the required JSON schema.".to_owned(),
            current_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            codex: config.codex.clone(),
            timeout: Duration::from_secs(config.timeout_seconds),
        },
        cancelled,
    ) {
        Ok(_) => {
            replace(
                checks,
                "auth",
                pass("live probe authenticated successfully"),
            );
            replace(
                checks,
                "model_reasoning",
                pass("configured model and reasoning are available"),
            );
            for id in [
                "codex.disable.shell_tool",
                "codex.disable.browser_use",
                "codex.disable.computer_use",
                "codex.disable.apps",
                "codex.project_doc_max_bytes",
                "codex.project_doc_fallback_filenames",
            ] {
                replace(checks, id, pass("exact value accepted by live invocation"));
            }
            pass("isolated Codex probe succeeded")
        }
        Err(error) => {
            let code = error.exit_code();
            match code {
                4 => replace(checks, "auth", fail(4, "Codex CLI is not authenticated")),
                5 => {
                    replace(
                        checks,
                        "auth",
                        pass("authentication accepted by live probe"),
                    );
                    replace(
                        checks,
                        "model_reasoning",
                        fail(5, "configured model or reasoning is unavailable"),
                    );
                }
                _ => {}
            }
            fail(code, "isolated Codex probe failed")
        }
    }
}

fn keymap_probe(shell: &Shell, config: &Config, cancelled: &AtomicBool) -> Check {
    let (program, args): (PathBuf, &[&str]) = match shell {
        Shell::Bash => (
            shell_executable(shell),
            &["--noprofile", "--norc", "-ic", "bind -q quoted-insert"],
        ),
        Shell::Zsh => (shell_executable(shell), &["-dfc", "bindkey '^G'"]),
        Shell::Fish => (
            shell_executable(shell),
            &["--no-config", "-ic", "bind -M insert \\cg"],
        ),
        Shell::Auto => return warn("keybinding shell is unknown"),
    };
    match bounded(&program, args, cancelled) {
        Probe::Ok(output) if output.trim().is_empty() => pass("no conflicting keybinding detected"),
        Probe::Ok(_) if config.keybinding.as_str() == "ctrl-g" => {
            warn("keybinding is occupied; inspect before enabling")
        }
        Probe::Ok(_) => warn("keybinding inspection returned an existing binding"),
        Probe::Cancelled => fail(7, "keybinding inspection cancelled"),
        Probe::Missing | Probe::Failed => warn("keybinding inspection could not run"),
    }
}

fn integration_check(shell: &Shell) -> Check {
    match IntegrationManager::from_system().and_then(|manager| manager.list()) {
        Ok(items)
            if items
                .iter()
                .any(|item| item.shell == *shell && item.is_current) =>
        {
            pass("current integration artifact installed")
        }
        Ok(items) if items.iter().any(|item| item.shell == *shell) => {
            warn("integration artifact version is mismatched")
        }
        Ok(_) => warn("integration artifact is absent"),
        Err(_) => fail(1, "integration artifact status is unreadable"),
    }
}
fn shell_version(shell: &Shell, cancelled: &AtomicBool) -> Probe {
    bounded(&shell_executable(shell), &["--version"], cancelled)
}
fn shell_executable(shell: &Shell) -> PathBuf {
    let key = format!("HASHAI_DOCTOR_{}_BIN", shell.as_str().to_ascii_uppercase());
    std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(shell.as_str()))
}
fn codex_executable() -> PathBuf {
    std::env::var_os("HASHAI_CODEX_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("codex"))
}

fn bounded(program: &PathBuf, args: &[&str], cancelled: &AtomicBool) -> Probe {
    if cancelled.load(Ordering::SeqCst) {
        return Probe::Cancelled;
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Probe::Missing,
        Err(_) => return Probe::Failed,
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => match child.wait_with_output() {
                Ok(output) if output.status.success() => {
                    return Probe::Ok(format!(
                        "{}{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
                _ => return Probe::Failed,
            },
            Ok(None) if cancelled.load(Ordering::SeqCst) => {
                terminate_bounded_group(&mut child);
                let _ = child.wait();
                return Probe::Cancelled;
            }
            Ok(None) if started.elapsed() >= Duration::from_secs(1) => {
                terminate_bounded_group(&mut child);
                let _ = child.wait();
                return Probe::Failed;
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => return Probe::Failed,
        }
    }
}
fn terminate_bounded_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}
fn version_at_least(text: &str, shell: &Shell) -> bool {
    let min = match shell {
        Shell::Bash => (5, 2),
        Shell::Zsh => (5, 8),
        Shell::Fish => (3, 6),
        Shell::Auto => return false,
    };
    let mut found = text
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .find(|part| part.contains('.'))
        .unwrap_or("")
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    matches!((found.next(), found.next()), (Some(a), Some(b)) if (a, b) >= min)
}
fn contains_any(text: &str, values: &[&str]) -> bool {
    let lower = text.to_ascii_lowercase();
    values.iter().any(|value| lower.contains(value))
}
fn has_token(text: &str, expected: &str) -> bool {
    text.split(|character: char| {
        !(character.is_ascii_alphanumeric() || character == '-' || character == '_')
    })
    .any(|token| token == expected)
}
fn named(id: &'static str, mut check: Check) -> Check {
    check.id = id;
    check
}
fn replace(checks: &mut [Check], id: &str, replacement: Check) {
    if let Some(check) = checks.iter_mut().find(|check| check.id == id) {
        *check = named(check.id, replacement);
    }
}
fn pass(message: &'static str) -> Check {
    Check {
        id: "",
        status: Status::Pass,
        message,
        exit: 0,
    }
}
fn warn(message: &'static str) -> Check {
    Check {
        id: "",
        status: Status::Warn,
        message,
        exit: 0,
    }
}
fn fail(exit: i32, message: &'static str) -> Check {
    Check {
        id: "",
        status: Status::Fail,
        message,
        exit,
    }
}
fn finalize(mode: &'static str, checks: Vec<Check>) -> Report {
    const PRECEDENCE: [i32; 8] = [9, 3, 4, 5, 6, 7, 8, 1];
    let exit = PRECEDENCE
        .into_iter()
        .find(|code| {
            checks
                .iter()
                .any(|check| check.status == Status::Fail && check.exit == *code)
        })
        .unwrap_or(0);
    let overall = if exit != 0 {
        Status::Fail
    } else if checks.iter().any(|check| check.status == Status::Warn) {
        Status::Warn
    } else {
        Status::Pass
    };
    Report {
        schema_version: SCHEMA_VERSION,
        mode,
        checks,
        overall,
        exit,
    }
}
