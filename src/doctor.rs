//! Bounded, redaction-safe readiness diagnostics for the hashai CLI.

use std::{
    fs::File,
    io::{ErrorKind, Read},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::{
    fd::{AsRawFd, FromRawFd},
    unix::process::CommandExt,
};

use serde::Serialize;

use crate::{
    config::{Config, Shell},
    integration::{IntegrationInspection, IntegrationManager, OwnershipState},
    runner::{CodexRunner, RunRequest},
};

pub const SCHEMA_VERSION: u8 = 2;
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
    "integration.artifact",
    "integration.startup_loader",
    "integration.startup_activation",
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

#[derive(Clone, Debug, Serialize)]
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
    Failed(String),
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
            Probe::Failed(String::new())
        } {
            Probe::Ok(text) if version_at_least(&text, &shell) => pass("supported shell version"),
            Probe::Ok(_) => fail(9, "unsupported shell version"),
            Probe::Missing | Probe::Failed(_) => fail(1, "shell version unavailable or invalid"),
            Probe::Cancelled => fail(7, "shell version inspection cancelled"),
        },
    ));

    let codex = codex_executable();
    let version = bounded(&codex, &["--version"], cancelled);
    let present =
        matches!(version, Probe::Ok(ref text) if normalized_codex_version(text).is_some());
    let version_exit = if matches!(version, Probe::Missing) {
        3
    } else {
        1
    };
    checks.push(named(
        "codex.command",
        match version {
            Probe::Ok(ref text) if normalized_codex_version(text).is_some() => {
                pass("Codex CLI is available")
            }
            Probe::Ok(_) => fail(1, "Codex CLI version is malformed"),
            Probe::Missing => fail(3, "Codex CLI is missing"),
            Probe::Cancelled => fail(7, "Codex CLI inspection cancelled"),
            Probe::Failed(_) => fail(1, "Codex CLI could not be inspected"),
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
                warn("requires strict behavioral verification; live invocation alone is insufficient")
            } else if !present {
                warn("Codex capability cannot be inspected while Codex is unavailable")
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
    let keybinding = if live {
        keymap_probe(&shell, config, cancelled)
    } else {
        warn("keybinding inspection requires --live")
    };
    let activation_proven = live && keybinding.status == Status::Pass;
    checks.push(named("keybinding", keybinding));
    let (artifact, loader, activation) = integration_checks(&shell, config, activation_proven);
    checks.push(named("integration.artifact", artifact));
    checks.push(named("integration.startup_loader", loader));
    checks.push(named("integration.startup_activation", activation));
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
        Probe::Failed(text)
            if contains_any(
                &text,
                &["not logged", "unauthenticated", "logged out", "codex login"],
            ) =>
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
            // Acceptance of the complete argv is useful evidence, but it does not
            // prove a particular disable/config control was honored. Those IDs stay
            // WARN until Codex offers a strict, per-setting acknowledgement.
            pass("one complete isolated Codex invocation succeeded")
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
    let artifact = match IntegrationManager::from_system()
        .and_then(|manager| manager.list_with_config(config))
    {
        Ok(items) => match items
            .into_iter()
            .find(|item| item.shell == *shell && item.is_current)
        {
            Some(item) => item.path,
            None => {
                return warn(
                    "current integration artifact is required for live keybinding inspection",
                );
            }
        },
        Err(_) => return warn("integration artifact could not be inspected for keybinding probe"),
    };
    let key = match (shell, config.keybinding.as_str()) {
        (Shell::Bash, "ctrl-g") => "\\C-g",
        (Shell::Bash, "ctrl-x") => "\\C-x",
        (Shell::Zsh, "ctrl-g") => "^G",
        (Shell::Zsh, "ctrl-x") => "^X",
        (Shell::Fish, "ctrl-g") => "ctrl-g",
        (Shell::Fish, "ctrl-x") => "ctrl-x",
        _ => return warn("keybinding shell is unknown"),
    };
    let script = keymap_harness_script(shell);
    let artifact = artifact.to_string_lossy();
    let args: Vec<&str> = match shell {
        // --live loads interactive startup files to observe the binding before
        // Hashai mutates it; the subprocess is still bounded and session-isolated.
        Shell::Bash => vec!["-ic", &script, "hashai-doctor", &artifact, key],
        Shell::Zsh => vec!["-ic", &script, "hashai-doctor", &artifact, key],
        Shell::Fish => vec!["-ic", &script, &artifact, key],
        Shell::Auto => unreachable!(),
    };
    match bounded_interactive(&shell_executable(shell), &args, cancelled) {
        Probe::Ok(output) => keymap_result(&output, config.trigger_enabled),
        Probe::Cancelled => fail(7, "keybinding inspection cancelled"),
        Probe::Missing | Probe::Failed(_) => warn("keybinding inspection could not run"),
    }
}

fn keymap_harness_script(shell: &Shell) -> String {
    match shell {
        Shell::Bash => r#"classify() { case "$1" in *'__hashai_bash_replace_line'*) printf owner;; '') printf unbound;; *) printf foreign;; esac; }
mapping() { { bind -S 2>/dev/null; bind -X 2>/dev/null; } | grep -F -- "\"$2\\000\""; }
pre=$(mapping "$1" "$2")
printf 'HASHAI_PRE:%s\n' "$(classify "$pre")"
source "$1"
post=$(mapping "$1" "$2")
printf 'HASHAI_POST:%s\n' "$(classify "$post")"
[[ $pre == "$post" ]] && printf 'HASHAI_UNCHANGED:yes\n' || printf 'HASHAI_UNCHANGED:no\n'"#.to_owned(),
        Shell::Zsh => r#"classify() { case "$1" in *'__hashai_zsh_replace_buffer'*) print -r -- owner;; *undefined-key*|'') print -r -- unbound;; *) print -r -- foreign;; esac; }
pre_emacs=$(bindkey -M emacs "$2")
pre_viins=$(bindkey -M viins "$2")
print -r -- "HASHAI_PRE:$(classify "$pre_emacs"),$(classify "$pre_viins")"
source "$1"
post_emacs=$(bindkey -M emacs "$2")
post_viins=$(bindkey -M viins "$2")
print -r -- "HASHAI_POST:$(classify "$post_emacs"),$(classify "$post_viins")"
[[ $pre_emacs == "$post_emacs" && $pre_viins == "$post_viins" ]] && print -r -- HASHAI_UNCHANGED:yes || print -r -- HASHAI_UNCHANGED:no"#.to_owned(),
        Shell::Fish => r#"function classify
    if string match -q '*__hashai_fish_replace_buffer*' -- $argv[1]
        echo owner
    else if test -z "$argv[1]"
        echo unbound
    else
        echo foreign
    end
end
emit fish_prompt
set -l pre_default (bind --user -M default $argv[2] 2>/dev/null)
set -l pre_insert (bind --user -M insert $argv[2] 2>/dev/null)
printf 'HASHAI_PRE:%s,%s\n' (classify "$pre_default") (classify "$pre_insert")
set -l post_default (bind --user -M default $argv[2] 2>/dev/null)
set -l post_insert (bind --user -M insert $argv[2] 2>/dev/null)
printf 'HASHAI_POST:%s,%s\n' (classify "$post_default") (classify "$post_insert")
if test "$pre_default" = "$post_default"; and test "$pre_insert" = "$post_insert"
    echo HASHAI_UNCHANGED:yes
else
    echo HASHAI_UNCHANGED:no
end"#.to_owned(),
        Shell::Auto => unreachable!(),
    }
}

fn keymap_result(output: &str, enabled: bool) -> Check {
    let pre = output
        .lines()
        .find_map(|line| line.strip_prefix("HASHAI_PRE:"));
    let post = output
        .lines()
        .find_map(|line| line.strip_prefix("HASHAI_POST:"));
    let unchanged = output
        .lines()
        .find_map(|line| line.strip_prefix("HASHAI_UNCHANGED:"));
    let Some(pre) = pre else {
        return warn("keybinding pre-source inspection was inconclusive");
    };
    let Some(post) = post else {
        return warn("keybinding post-source ownership check was inconclusive");
    };
    let Some(unchanged) = unchanged else {
        return warn("keybinding pre/post comparison was inconclusive");
    };
    if !enabled {
        if unchanged == "yes"
            && pre.split(',').all(|state| state != "owner")
            && post.split(',').all(|state| state != "owner")
        {
            return pass("disabled integration leaves the configured keybinding unchanged");
        }
        return warn("disabled integration changed or claimed the configured keybinding");
    }
    if pre.split(',').any(|state| state == "foreign") {
        return warn("configured keybinding is occupied before Hashai loads");
    }
    if pre
        .split(',')
        .any(|state| !matches!(state, "unbound" | "owner"))
    {
        return warn("keybinding pre-source inspection was inconclusive");
    }
    if post.split(',').all(|state| state == "owner") {
        pass(
            "configured keybinding was unbound or Hashai-owned before loading and is Hashai-owned after loading",
        )
    } else {
        warn("configured keybinding ownership was not confirmed after loading the artifact")
    }
}

fn integration_checks(
    shell: &Shell,
    config: &Config,
    activation_proven: bool,
) -> (Check, Check, Check) {
    let inspection = IntegrationManager::from_system()
        .and_then(|manager| manager.inspect(shell, config))
        .unwrap_or(IntegrationInspection {
            artifact: OwnershipState::Unreadable,
            loader: Some(OwnershipState::Unreadable),
            desired_mode: None,
        });
    let artifact = match inspection.artifact {
        OwnershipState::Absent => warn("absent"),
        OwnershipState::UntrackedExactExpected => warn("untracked-expected"),
        OwnershipState::TrackedExact => pass("current"),
        OwnershipState::TrackedPrior => warn("prior-supported"),
        OwnershipState::Modified => warn("modified"),
        OwnershipState::Foreign => warn("foreign"),
        OwnershipState::Unsafe => fail(1, "unsafe"),
        OwnershipState::Unreadable => fail(1, "unreadable"),
        OwnershipState::InterruptedRecoverable => warn("interrupted-recoverable"),
    };
    if *shell != Shell::Fish {
        return (artifact, warn("manual-startup"), warn("manual-startup"));
    }
    let loader_state = inspection.loader.unwrap_or(OwnershipState::Absent);
    if matches!(
        loader_state,
        OwnershipState::Unsafe | OwnershipState::Unreadable
    ) {
        return (
            artifact,
            fail(1, "loader-unsafe-or-unreadable"),
            warn("artifact-not-evaluated"),
        );
    }
    if artifact.status == Status::Fail {
        return (
            artifact,
            warn("artifact-not-evaluated"),
            warn("artifact-not-evaluated"),
        );
    }
    let loader = match loader_state {
        OwnershipState::TrackedExact => pass("loader-state"),
        _ => warn("loader-state"),
    };
    let activation = if loader.status == Status::Pass && activation_proven {
        pass("current-and-active")
    } else {
        warn("loader-state")
    };
    (artifact, loader, activation)
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

#[cfg(unix)]
fn bounded_interactive(program: &PathBuf, args: &[&str], cancelled: &AtomicBool) -> Probe {
    if cancelled.load(Ordering::SeqCst) {
        return Probe::Cancelled;
    }
    let mut master = -1;
    let mut slave = -1;
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } != 0
    {
        return Probe::Failed(String::new());
    }
    let duplicate = |fd| unsafe { libc::dup(fd) };
    let stdin = duplicate(slave);
    let stdout = duplicate(slave);
    let stderr = duplicate(slave);
    unsafe { libc::close(slave) };
    if stdin < 0 || stdout < 0 || stderr < 0 {
        unsafe {
            libc::close(master);
            for fd in [stdin, stdout, stderr] {
                if fd >= 0 {
                    libc::close(fd);
                }
            }
        }
        return Probe::Failed(String::new());
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader_stop = Arc::new(AtomicBool::new(false));
    let reader_stop_for_thread = Arc::clone(&reader_stop);
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let file = unsafe { File::from_raw_fd(master) };
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
        loop {
            let mut chunk = [0; 4096];
            match (&file).read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => output.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if reader_stop_for_thread.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
        let _ = sender.send(output);
    });
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(unsafe { Stdio::from(File::from_raw_fd(stdin)) })
        .stdout(unsafe { Stdio::from(File::from_raw_fd(stdout)) })
        .stderr(unsafe { Stdio::from(File::from_raw_fd(stderr)) });
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as libc::c_ulong, 0) == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Probe::Missing,
        Err(_) => return Probe::Failed(String::new()),
    };
    drop(command);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if cancelled.load(Ordering::SeqCst) => {
                terminate_bounded_group(&mut child);
                let _ = child.wait();
                break Err(Probe::Cancelled);
            }
            Ok(None) if started.elapsed() >= Duration::from_secs(1) => {
                terminate_bounded_group(&mut child);
                let _ = child.wait();
                break Err(Probe::Failed(String::new()));
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => break Err(Probe::Failed(String::new())),
        }
    };
    // The shell leader may successfully exit while a startup hook leaves a
    // descendant attached to the PTY. Never let that descendant keep doctor
    // alive or retain the terminal: drain the whole session before consuming
    // the reader result.
    drain_bounded_group(child.id() as i32);
    reader_stop.store(true, Ordering::SeqCst);
    let output = receiver
        .recv_timeout(Duration::from_millis(250))
        .unwrap_or_default();
    let _ = reader.join();
    let output = String::from_utf8_lossy(&output).into_owned();
    match status {
        Ok(status) if status.success() => Probe::Ok(output),
        Ok(_) => Probe::Failed(output),
        Err(probe) => probe,
    }
}

#[cfg(not(unix))]
fn bounded_interactive(program: &PathBuf, args: &[&str], cancelled: &AtomicBool) -> Probe {
    bounded(program, args, cancelled)
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
        Err(_) => return Probe::Failed(String::new()),
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
                Ok(output) => {
                    return Probe::Failed(format!(
                        "{}{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
                Err(_) => return Probe::Failed(String::new()),
            },
            Ok(None) if cancelled.load(Ordering::SeqCst) => {
                terminate_bounded_group(&mut child);
                let _ = child.wait();
                return Probe::Cancelled;
            }
            Ok(None) if started.elapsed() >= Duration::from_secs(1) => {
                terminate_bounded_group(&mut child);
                let _ = child.wait();
                return Probe::Failed(String::new());
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => return Probe::Failed(String::new()),
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

#[cfg(unix)]
fn drain_bounded_group(group: i32) {
    unsafe {
        let _ = libc::kill(-group, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_millis(100);
    while Instant::now() < deadline {
        let exists = unsafe { libc::kill(-group, 0) } == 0;
        if !exists {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    unsafe {
        let _ = libc::kill(-group, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn drain_bounded_group(_: i32) {}
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
fn normalized_codex_version(text: &str) -> Option<(u32, u32, u32)> {
    let candidate = text
        .split_whitespace()
        .find(|token| token.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut parts = candidate.split('.').map(str::parse::<u32>);
    match (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next(),
    ) {
        (major, minor, patch, None) => Some((major, minor, patch)),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_exit_precedence_pair_uses_the_documented_winner() {
        let precedence = [9, 3, 4, 5, 6, 7, 8, 1];
        for (winner_index, winner) in precedence.iter().enumerate() {
            for loser in precedence.iter().skip(winner_index + 1) {
                let report = finalize(
                    "static",
                    vec![
                        named("platform", fail(*winner, "winner")),
                        named("auth", fail(*loser, "loser")),
                    ],
                );
                assert_eq!(report.exit, *winner, "{winner} must win over {loser}");
            }
        }
    }

    #[test]
    fn normalized_codex_version_rejects_partial_and_extra_components() {
        assert_eq!(normalized_codex_version("codex 1.2.3"), Some((1, 2, 3)));
        for malformed in ["codex 1.2", "codex 1.2.3.4", "codex v1.2.3", "unknown"] {
            assert_eq!(normalized_codex_version(malformed), None, "{malformed}");
        }
    }

    #[test]
    fn keymap_result_separates_pre_source_conflicts_from_post_source_ownership() {
        assert_eq!(
            keymap_result(
                "HASHAI_PRE:unbound,owner\nHASHAI_POST:owner,owner\nHASHAI_UNCHANGED:no\n",
                true
            )
            .status,
            Status::Pass
        );
        assert_eq!(
            keymap_result(
                "HASHAI_PRE:foreign,unbound\nHASHAI_POST:owner,owner\nHASHAI_UNCHANGED:no\n",
                true
            )
            .status,
            Status::Warn
        );
        assert_eq!(
            keymap_result(
                "HASHAI_PRE:unbound\nHASHAI_POST:unbound\nHASHAI_UNCHANGED:yes\n",
                false
            )
            .status,
            Status::Pass
        );
        assert_eq!(
            keymap_result(
                "HASHAI_PRE:unbound\nHASHAI_POST:owner\nHASHAI_UNCHANGED:no\n",
                false
            )
            .status,
            Status::Warn
        );
        assert_eq!(
            keymap_result(
                "HASHAI_PRE:foreign\nHASHAI_POST:foreign\nHASHAI_UNCHANGED:yes\n",
                false
            )
            .status,
            Status::Pass
        );
    }

    #[test]
    fn keymap_harnesses_query_every_required_map_before_sourcing() {
        let bash = keymap_harness_script(&Shell::Bash);
        assert!(bash.contains("bind -S") && bash.contains("bind -X"));
        assert!(bash.find("pre=").unwrap() < bash.find("source").unwrap());
        let zsh = keymap_harness_script(&Shell::Zsh);
        assert!(zsh.contains("-M emacs") && zsh.contains("-M viins"));
        assert!(zsh.find("pre_emacs").unwrap() < zsh.find("source").unwrap());
        let fish = keymap_harness_script(&Shell::Fish);
        assert!(fish.contains("-M default") && fish.contains("-M insert"));
        assert!(fish.find("emit fish_prompt").unwrap() < fish.find("pre_default").unwrap());
        assert!(!fish.contains("source $argv[1]"));
    }

    #[cfg(unix)]
    #[test]
    fn interactive_probe_reaps_a_startup_background_descendant_without_hanging() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("background-pid");
        let script = format!("sleep 30 & echo $! > {}; exit 0", marker.display());
        let cancelled = AtomicBool::new(false);
        let started = Instant::now();
        assert!(matches!(
            bounded_interactive(
                &PathBuf::from("/bin/sh"),
                &["-c", script.as_str()],
                &cancelled
            ),
            Probe::Ok(_)
        ));
        assert!(started.elapsed() < Duration::from_secs(1));
        let pid: i32 = std::fs::read_to_string(marker)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_millis(250);
        while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "startup descendant survived"
        );
    }
}
