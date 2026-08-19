use std::{
    fs,
    io::Write,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;
use tempfile::NamedTempFile;

use crate::{HashaiError, config::CodexConfig};

const OUTPUT_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "command": { "type": "string", "minLength": 1 },
    "risk": { "type": "string", "enum": ["safe", "review", "dangerous"] }
  },
  "required": ["command", "risk"],
  "additionalProperties": false
}"#;

#[derive(Debug)]
pub struct RunRequest {
    pub executable: PathBuf,
    pub prompt: String,
    pub current_dir: PathBuf,
    pub codex: CodexConfig,
    pub timeout: Duration,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Risk {
    Safe,
    Review,
    Dangerous,
}

impl Risk {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Review => "review",
            Self::Dangerous => "dangerous",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct Generation {
    pub command: String,
    pub risk: Risk,
}

#[derive(Default)]
pub struct CodexRunner;

impl CodexRunner {
    pub fn new() -> Self {
        Self
    }

    pub fn run(
        &self,
        request: RunRequest,
        cancelled: &AtomicBool,
    ) -> Result<Generation, HashaiError> {
        if cancelled.load(Ordering::SeqCst) {
            return Err(HashaiError::Cancelled(
                "Codex generation was cancelled".to_owned(),
            ));
        }

        let mut schema = private_temp_file()?;
        schema.write_all(OUTPUT_SCHEMA.as_bytes())?;
        schema.flush()?;
        let output = private_temp_file()?;
        let stderr = private_temp_file()?;
        let stderr_writer = stderr.reopen()?;

        let mut command = Command::new(&request.executable);
        command
            .arg("exec")
            .arg("-")
            .args(["--ephemeral", "--ignore-user-config", "--ignore-rules"])
            .arg("--model")
            .arg(&request.codex.model)
            .arg("--config")
            .arg(format!(
                "model_reasoning_effort=\"{}\"",
                request.codex.reasoning_effort
            ))
            .args(["--config", "project_doc_max_bytes=0"])
            .args(["--config", "project_doc_fallback_filenames=[]"])
            .args(["--sandbox", "read-only"])
            .args(["--disable", "shell_tool"])
            .args(["--disable", "browser_use"])
            .args(["--disable", "computer_use"])
            .args(["--disable", "apps"])
            .arg("--skip-git-repo-check")
            .arg("--output-schema")
            .arg(schema.path())
            .arg("--output-last-message")
            .arg(output.path())
            .current_dir(&request.current_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr_writer));
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }

        let mut child = command.spawn().map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => HashaiError::CodexNotFound(format!(
                "Codex CLI was not found at {}; install Codex CLI and run `codex login`",
                request.executable.display()
            )),
            _ => HashaiError::Io(std::io::Error::new(
                error.kind(),
                format!(
                    "failed to start Codex CLI at {}: {error}",
                    request.executable.display()
                ),
            )),
        })?;
        if let Some(mut stdin) = child.stdin.take()
            && let Err(error) = stdin.write_all(request.prompt.as_bytes())
        {
            terminate_process_group(&mut child)?;
            return Err(error.into());
        }

        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if cancelled.load(Ordering::SeqCst) {
                terminate_process_group(&mut child)?;
                return Err(HashaiError::Cancelled(
                    "Codex generation was cancelled".to_owned(),
                ));
            }
            if started.elapsed() >= request.timeout {
                terminate_process_group(&mut child)?;
                return Err(HashaiError::Timeout(format!(
                    "Codex generation exceeded the {} second timeout",
                    request.timeout.as_secs_f64()
                )));
            }
            thread::sleep(Duration::from_millis(10));
        };

        if !status.success() {
            let stderr = fs::read(stderr.path()).unwrap_or_default();
            let output = fs::read(output.path()).unwrap_or_default();
            return Err(classify_stderr(&stderr, &status, output.len()));
        }
        parse_generation(&fs::read_to_string(output.path())?)
    }
}

fn private_temp_file() -> Result<NamedTempFile, HashaiError> {
    let file = NamedTempFile::new()?;
    fs::set_permissions(file.path(), fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

fn terminate_process_group(child: &mut std::process::Child) -> Result<(), HashaiError> {
    let group = child.id() as i32;
    let mut leader_reaped = false;

    signal_process_group(group, libc::SIGTERM)?;
    if !wait_for_process_group_exit(
        child,
        group,
        Instant::now() + Duration::from_millis(200),
        &mut leader_reaped,
    )? {
        signal_process_group(group, libc::SIGKILL)?;
        // SIGKILL has been sent to every member of the process group. Do not
        // wait for `kill(-group, 0)` to become ESRCH here: an orphaned child
        // can remain a zombie until init reaps it, and zombies are already
        // dead even though kill(2) still reports them as existing.
        wait_for_leader_exit(
            child,
            Instant::now() + Duration::from_secs(2),
            &mut leader_reaped,
        )?;
        if !leader_reaped {
            return Err(HashaiError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Codex process leader did not exit after SIGKILL",
            )));
        }
    }

    if !leader_reaped {
        wait_for_leader_exit(
            child,
            Instant::now() + Duration::from_secs(2),
            &mut leader_reaped,
        )?;
        if !leader_reaped {
            return Err(HashaiError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Codex process leader did not exit after termination",
            )));
        }
    }
    Ok(())
}

fn wait_for_leader_exit(
    child: &mut std::process::Child,
    deadline: Instant,
    leader_reaped: &mut bool,
) -> Result<(), HashaiError> {
    while !*leader_reaped && Instant::now() < deadline {
        *leader_reaped = child.try_wait()?.is_some();
        if !*leader_reaped {
            thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}

fn signal_process_group(group: i32, signal: i32) -> Result<(), HashaiError> {
    let result = unsafe { libc::kill(-group, signal) };
    if result == -1 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error.into());
        }
    }
    Ok(())
}

fn wait_for_process_group_exit(
    child: &mut std::process::Child,
    group: i32,
    deadline: Instant,
    leader_reaped: &mut bool,
) -> Result<bool, HashaiError> {
    loop {
        if !*leader_reaped {
            *leader_reaped = child.try_wait()?.is_some();
        }
        if !process_group_exists(group)? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn process_group_exists(group: i32) -> Result<bool, HashaiError> {
    let result = unsafe { libc::kill(-group, 0) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(error.into()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexOutput {
    command: String,
    risk: Risk,
}

impl<'de> Deserialize<'de> for Risk {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match String::deserialize(deserializer)?.as_str() {
            "safe" => Ok(Self::Safe),
            "review" => Ok(Self::Review),
            "dangerous" => Ok(Self::Dangerous),
            _ => Err(serde::de::Error::custom(
                "risk must be safe, review, or dangerous",
            )),
        }
    }
}

fn parse_generation(output: &str) -> Result<Generation, HashaiError> {
    let parsed: CodexOutput = serde_json::from_str(output).map_err(|error| {
        HashaiError::InvalidOutput(format!("Codex returned invalid structured output: {error}"))
    })?;
    if parsed.command.trim().is_empty() {
        return Err(HashaiError::InvalidOutput(
            "Codex returned an empty command".to_owned(),
        ));
    }
    Ok(Generation {
        command: parsed.command,
        risk: parsed.risk,
    })
}

fn classify_stderr(stderr: &[u8], status: &ExitStatus, output_bytes: usize) -> HashaiError {
    let stderr = String::from_utf8_lossy(stderr);
    let normalized = stderr.to_ascii_lowercase();
    if normalized.contains("codex login")
        || normalized.contains("not logged in")
        || normalized.contains("unauthenticated")
    {
        HashaiError::Unauthenticated("Codex CLI is not authenticated; run `codex login`".to_owned())
    } else if (normalized.contains("model") || normalized.contains("reasoning"))
        && (normalized.contains("unavailable")
            || normalized.contains("not available")
            || normalized.contains("unsupported"))
    {
        HashaiError::ModelUnavailable(
            "The configured Codex model or reasoning effort is unavailable; update hashai configuration".to_owned(),
        )
    } else {
        HashaiError::Io(std::io::Error::other(format!(
            "Codex CLI failed with {status}; stderr bytes: {}; output-file bytes: {output_bytes}",
            stderr.len()
        )))
    }
}
