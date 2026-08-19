#![cfg(unix)]

use std::process::Command;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use hashai::{
    ExitCode,
    config::CodexConfig,
    runner::{CodexRunner, RunRequest},
};
use tempfile::TempDir;

const WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const DRAIN_STDIN: &str = "while IFS= read -r line || [ -n \"${line:-}\" ]; do :; done";
static NEXT_FAKE: AtomicUsize = AtomicUsize::new(0);

fn fake(temp: &TempDir, body: &str) -> PathBuf {
    let path = temp.path().join(format!(
        "fake-codex-{}",
        NEXT_FAKE.fetch_add(1, Ordering::Relaxed)
    ));
    let staging = temp.path().join(format!(
        ".{}-staging",
        path.file_name().unwrap().to_string_lossy()
    ));
    fs::write(&staging, ["#!/bin/sh", "set -eu", body, ""].join("\n")).unwrap();
    let mut permissions = fs::metadata(&staging).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&staging, permissions).unwrap();
    fs::rename(staging, &path).unwrap();
    path
}

#[test]
fn fake_executables_do_not_reuse_a_writable_path() {
    let temp = TempDir::new().unwrap();
    let first = fake(&temp, "exit 0");
    let second = fake(&temp, "exit 0");
    assert_ne!(first, second);
    assert!(first.exists());
    assert!(second.exists());
}

#[test]
fn fake_publication_replaces_an_executing_script_via_closed_staging_file() {
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("fake-codex");
    let started = temp.path().join("started");
    let initial_staging = temp.path().join(".fake-codex-initial-staging");
    fs::write(
        &initial_staging,
        format!("#!/bin/sh\nprintf x > {}\nsleep 2\n", started.display()),
    )
    .unwrap();
    let mut permissions = fs::metadata(&initial_staging).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&initial_staging, permissions).unwrap();
    fs::rename(&initial_staging, &executable).unwrap();
    let mut running = Command::new(&executable).spawn().unwrap();
    wait_for_file(&started, "executing fake marker");

    let staging = temp.path().join(".fake-codex-staging");
    fs::write(&staging, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = fs::metadata(&staging).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&staging, permissions).unwrap();
    fs::rename(staging, &executable).unwrap();
    assert!(Command::new(&executable).status().unwrap().success());
    let _ = running.wait();
}

fn request(executable: PathBuf, temp: &TempDir) -> RunRequest {
    RunRequest {
        executable,
        prompt: "list Japanese files 日本語".to_owned(),
        current_dir: temp.path().to_path_buf(),
        codex: CodexConfig {
            model: "test-model".to_owned(),
            reasoning_effort: "low".to_owned(),
        },
        timeout: Duration::from_secs(2),
    }
}

fn wait_for_file(path: &Path, description: &str) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}: {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn assert_process_gone(pid: i32) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        if unsafe { libc::kill(pid, 0) } == -1 || process_is_zombie(pid) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "descendant {pid} survived process-group termination"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "linux")]
fn process_is_zombie(pid: i32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(") ")
                .map(|(_, state)| state.starts_with('Z'))
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn process_is_zombie(pid: i32) -> bool {
    Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .ok()
        .is_some_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim_start()
                .starts_with('Z')
        })
}

#[test]
fn ac1_ac2_ac3_passes_stdin_cwd_fixed_arguments_and_private_temp_files() {
    let temp = TempDir::new().unwrap();
    let capture = temp.path().join("capture");
    let body = [
        format!(r#"printf '%s\n' "$PWD" > "{}""#, capture.display()),
        format!(r#"printf '%s\n' "$@" >> "{}""#, capture.display()),
        format!(r#"cat >> "{}""#, capture.display()),
        "schema=''; out=''".to_owned(),
        "while [ \"$#\" -gt 0 ]; do case \"$1\" in --output-schema) schema=\"$2\"; shift 2;; --output-last-message) out=\"$2\"; shift 2;; *) shift;; esac; done".to_owned(),
        "file_mode() { if [ \"$(uname)\" = Darwin ]; then stat -f '%Lp' \"$1\"; else stat -c '%a' \"$1\"; fi; }".to_owned(),
        "[ \"$(file_mode \"$schema\")\" = 600 ]".to_owned(),
        "[ \"$(file_mode \"$out\")\" = 600 ]".to_owned(),
        r#"printf '%s' '{"command":"find . -type f","risk":"safe"}' > "$out""#.to_owned(),
    ].join("\n");
    let result = CodexRunner::new()
        .run(request(fake(&temp, &body), &temp), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.command, "find . -type f");
    assert_eq!(result.risk.as_str(), "safe");
    let captured = fs::read_to_string(capture).unwrap();
    let lines: Vec<_> = captured.lines().collect();
    assert_eq!(
        lines[0],
        fs::canonicalize(temp.path()).unwrap().display().to_string()
    );
    assert_eq!(
        &lines[1..26],
        [
            "exec",
            "-",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--model",
            "test-model",
            "--config",
            "model_reasoning_effort=\"low\"",
            "--config",
            "project_doc_max_bytes=0",
            "--config",
            "project_doc_fallback_filenames=[]",
            "--sandbox",
            "read-only",
            "--disable",
            "shell_tool",
            "--disable",
            "browser_use",
            "--disable",
            "computer_use",
            "--disable",
            "apps",
            "--skip-git-repo-check",
            "--output-schema",
        ]
    );
    assert!(!lines[26].is_empty());
    assert_eq!(lines[27], "--output-last-message");
    assert!(!lines[28].is_empty());
    assert_eq!(lines[29], "list Japanese files 日本語");
}

#[test]
fn ac4_rejects_empty_invalid_and_schema_violating_output() {
    for output in [
        "",
        "not json",
        r#"{"command":"ok","risk":"safe","extra":true}"#,
        r#"{"command":" ","risk":"safe"}"#,
    ] {
        let temp = TempDir::new().unwrap();
        let body = [
            "out=''",
            "while [ \"$#\" -gt 0 ]; do case \"$1\" in --output-last-message) out=\"$2\"; shift 2;; *) shift;; esac; done",
            DRAIN_STDIN,
            &format!("printf '%s' '{output}' > \"$out\""),
        ]
        .join("\n");
        let error = CodexRunner::new()
            .run(request(fake(&temp, &body), &temp), &AtomicBool::new(false))
            .unwrap_err();
        assert_eq!(
            error.exit_code(),
            ExitCode::InvalidOutput as i32,
            "{output:?}: {error}"
        );
    }
}

#[test]
fn ac4_fixture_drains_stdin_without_an_external_cat() {
    let temp = TempDir::new().unwrap();
    let body = [
        "PATH=/definitely-no-cat",
        "out=''",
        "while [ \"$#\" -gt 0 ]; do case \"$1\" in --output-last-message) out=\"$2\"; shift 2;; *) shift;; esac; done",
        DRAIN_STDIN,
        "printf '%s' 'not json' > \"$out\"",
    ]
    .join("\n");
    let error = CodexRunner::new()
        .run(request(fake(&temp, &body), &temp), &AtomicBool::new(false))
        .unwrap_err();
    assert_eq!(error.exit_code(), ExitCode::InvalidOutput as i32);
}

#[test]
fn ac5_classifies_known_errors_and_leaves_unknown_as_general() {
    for (stderr, expected) in [
        ("Please run codex login", ExitCode::Unauthenticated),
        ("selected model is unavailable", ExitCode::ModelUnavailable),
        ("an unknown failure", ExitCode::General),
    ] {
        let temp = TempDir::new().unwrap();
        let error = CodexRunner::new()
            .run(
                request(
                    fake(
                        &temp,
                        &format!("{DRAIN_STDIN}; echo '{stderr}' >&2; exit 1"),
                    ),
                    &temp,
                ),
                &AtomicBool::new(false),
            )
            .unwrap_err();
        assert_eq!(error.exit_code(), expected as i32, "{stderr}: {error}");
    }
    let temp = TempDir::new().unwrap();
    let error = CodexRunner::new()
        .run(
            request(temp.path().join("absent-codex"), &temp),
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert_eq!(error.exit_code(), ExitCode::CodexNotFound as i32);
}

#[test]
fn ac5_unclassified_process_failure_reports_status_and_artifact_sizes() {
    let temp = TempDir::new().unwrap();
    let body = [
        "out=''",
        "while [ \"$#\" -gt 0 ]; do case \"$1\" in --output-last-message) out=\"$2\"; shift 2;; *) shift;; esac; done",
        DRAIN_STDIN,
        "printf x > \"$out\"",
        "printf error >&2",
        "exit 9",
    ]
    .join("\n");
    let error = CodexRunner::new()
        .run(request(fake(&temp, &body), &temp), &AtomicBool::new(false))
        .unwrap_err();
    assert_eq!(error.exit_code(), ExitCode::General as i32);
    assert!(
        error.to_string().contains("exit status: 9"),
        "unexpected unclassified error: {error}"
    );
    assert!(error.to_string().contains("stderr bytes: 5"));
    assert!(error.to_string().contains("output-file bytes: 1"));
}

#[test]
fn parallel_invalid_general_timeout_and_cancel_keep_their_classifications() {
    for _round in 0..6 {
        let barrier = Barrier::new(4);
        let (invalid, general, timeout, cancelled) = thread::scope(|scope| {
            let invalid = scope.spawn(|| {
                let temp = TempDir::new().unwrap();
                let body = [
                    "out=''",
                    "while [ \"$#\" -gt 0 ]; do case \"$1\" in --output-last-message) out=\"$2\"; shift 2;; *) shift;; esac; done",
                    DRAIN_STDIN,
                    "printf '%s' 'not json' > \"$out\"",
                ]
                .join("\n");
                barrier.wait();
                CodexRunner::new()
                    .run(request(fake(&temp, &body), &temp), &AtomicBool::new(false))
                    .unwrap_err()
                    .exit_code()
            });
            let general = scope.spawn(|| {
                let temp = TempDir::new().unwrap();
                barrier.wait();
                CodexRunner::new()
                    .run(
                        request(fake(&temp, &format!("{DRAIN_STDIN}; exit 1")), &temp),
                        &AtomicBool::new(false),
                    )
                    .unwrap_err()
                    .exit_code()
            });
            let timeout = scope.spawn(|| {
                let temp = TempDir::new().unwrap();
                let mut task = request(fake(&temp, "trap '' TERM; sleep 30"), &temp);
                task.timeout = Duration::from_millis(100);
                barrier.wait();
                CodexRunner::new()
                    .run(task, &AtomicBool::new(false))
                    .unwrap_err()
                    .exit_code()
            });
            let cancelled = scope.spawn(|| {
                let temp = TempDir::new().unwrap();
                let marker = temp.path().join("started");
                let fake = fake(
                    &temp,
                    &format!("printf x > {}; trap '' TERM; sleep 30", marker.display()),
                );
                let flag = AtomicBool::new(false);
                let task = request(fake, &temp);
                barrier.wait();
                thread::scope(|scope| {
                    scope.spawn(|| {
                        wait_for_file(&marker, "cancel marker");
                        flag.store(true, Ordering::SeqCst);
                    });
                    CodexRunner::new().run(task, &flag).unwrap_err().exit_code()
                })
            });
            (
                invalid.join().unwrap(),
                general.join().unwrap(),
                timeout.join().unwrap(),
                cancelled.join().unwrap(),
            )
        });
        assert_eq!(invalid, ExitCode::InvalidOutput as i32);
        assert_eq!(general, ExitCode::General as i32);
        assert_eq!(timeout, ExitCode::Timeout as i32);
        assert_eq!(cancelled, ExitCode::Cancelled as i32);
    }
}

#[test]
fn ac7_removes_private_temp_files_after_success_and_error() {
    for exit_code in [0, 1] {
        let temp = TempDir::new().unwrap();
        let paths = temp.path().join("paths");
        let body = [
            "schema=''; out=''",
            "while [ \"$#\" -gt 0 ]; do case \"$1\" in --output-schema) schema=\"$2\"; shift 2;; --output-last-message) out=\"$2\"; shift 2;; *) shift;; esac; done",
            DRAIN_STDIN,
            &format!("printf '%s\\n%s\\n' \"$schema\" \"$out\" > {}", paths.display()),
            "printf '%s' '{\"command\":\"echo ok\",\"risk\":\"safe\"}' > \"$out\"",
            &format!("exit {exit_code}"),
        ]
        .join("\n");
        let result =
            CodexRunner::new().run(request(fake(&temp, &body), &temp), &AtomicBool::new(false));
        let recorded_paths = fs::read_to_string(&paths).unwrap_or_else(|error| {
            panic!("temporary-file marker was not written after {result:?}: {error}")
        });
        for path in recorded_paths.lines() {
            assert!(
                !Path::new(path).exists(),
                "temporary file was retained: {path}"
            );
        }
    }
}

#[test]
fn ac7_removes_private_temp_files_after_timeout_and_cancel() {
    for cancelled_case in [false, true] {
        let temp = TempDir::new().unwrap();
        let paths = temp.path().join("paths");
        let body = ["schema=''; out=''", "while [ \"$#\" -gt 0 ]; do case \"$1\" in --output-schema) schema=\"$2\"; shift 2;; --output-last-message) out=\"$2\"; shift 2;; *) shift;; esac; done", &format!("printf '%s\\n%s\\n' \"$schema\" \"$out\" > {}", paths.display()), "sleep 30"].join("\n");
        let cancelled = AtomicBool::new(false);
        let mut task = request(fake(&temp, &body), &temp);
        task.timeout = Duration::from_millis(100);
        let result = thread::scope(|scope| {
            if cancelled_case {
                scope.spawn(|| {
                    wait_for_file(&paths, "temporary-file marker");
                    cancelled.store(true, Ordering::SeqCst);
                });
            }
            CodexRunner::new().run(task, &cancelled)
        });
        let error = result.unwrap_err();
        assert_eq!(
            error.exit_code(),
            if cancelled_case {
                ExitCode::Cancelled as i32
            } else {
                ExitCode::Timeout as i32
            },
            "unexpected timeout/cancel result: {}",
            error
        );
        for path in fs::read_to_string(paths).unwrap().lines() {
            assert!(
                !Path::new(path).exists(),
                "temporary file was retained: {path}"
            );
        }
    }
}

#[test]
fn ac6_timeout_escalates_past_a_term_ignoring_descendant() {
    let temp = TempDir::new().unwrap();
    let descendant = temp.path().join("descendant.pid");
    let fake = fake(
        &temp,
        &format!(
            "(trap '' TERM; exec sleep 30) & echo $! > {}; wait",
            descendant.display()
        ),
    );
    let mut task = request(fake, &temp);
    task.timeout = Duration::from_millis(100);
    let error = CodexRunner::new()
        .run(task, &AtomicBool::new(false))
        .unwrap_err();
    assert_eq!(error.exit_code(), ExitCode::Timeout as i32);
    wait_for_file(&descendant, "timeout descendant marker");
    let pid = fs::read_to_string(descendant)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    assert_process_gone(pid);
}

#[test]
fn ac6_cancel_escalates_past_a_term_ignoring_descendant() {
    let temp = TempDir::new().unwrap();
    let descendant = temp.path().join("descendant.pid");
    let fake = fake(
        &temp,
        &format!(
            "(trap '' TERM; exec sleep 30) & echo $! > {}; wait",
            descendant.display()
        ),
    );
    let cancelled = AtomicBool::new(false);
    let result = thread::scope(|scope| {
        scope.spawn(|| {
            wait_for_file(&descendant, "cancel descendant marker");
            cancelled.store(true, Ordering::SeqCst);
        });
        CodexRunner::new().run(request(fake, &temp), &cancelled)
    });
    assert_eq!(result.unwrap_err().exit_code(), ExitCode::Cancelled as i32);
    let pid = fs::read_to_string(descendant)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    assert_process_gone(pid);
}
