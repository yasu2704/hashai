#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use assert_cmd::Command as AssertCommand;
use hashai::ExitCode;
use tempfile::TempDir;

const WAIT_TIMEOUT: Duration = Duration::from_secs(2);

fn fake_codex(temp: &TempDir, body: &str) -> PathBuf {
    let path = temp.path().join("fake-codex");
    fs::write(&path, ["#!/bin/sh", "set -eu", body, ""].join("\n")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn json_response(command: &str, risk: &str) -> String {
    serde_json::json!({ "command": command, "risk": risk }).to_string()
}

fn output_file_writer(response: &str) -> String {
    format!(
        "out=''\nwhile [ \"$#\" -gt 0 ]; do case \"$1\" in --output-last-message) out=\"$2\"; shift 2;; *) shift;; esac; done\ncat >/dev/null\ncat > \"$out\" <<'HASHAI_JSON'\n{response}\nHASHAI_JSON"
    )
}

fn command_with(temp: &TempDir, executable: &Path) -> AssertCommand {
    let config_home = temp.path().join("config-home");
    fs::create_dir(&config_home).unwrap();
    let mut command = AssertCommand::cargo_bin("hashai").unwrap();
    command
        .env("XDG_CONFIG_HOME", config_home)
        .env("HASHAI_CODEX_BIN", executable)
        .env_remove("HASHAI_TRIGGER")
        .env_remove("HASHAI_TIMEOUT_SECONDS")
        .env_remove("HASHAI_SHELL")
        .env_remove("HASHAI_CODEX_MODEL")
        .env_remove("HASHAI_CODEX_REASONING_EFFORT");
    command
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

fn wait_for_child_exit(child: &mut Child, description: &str) -> ExitStatus {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("timed out waiting for {description}");
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn ac1_ac2_ac3_generate_uses_fake_codex_and_keeps_command_stdout_pure() {
    let temp = TempDir::new().unwrap();
    let command = "  printf '%s\\n' '日本語 😀' &&\nprintf '%s' \"quoted value  \"  ";
    let fake = fake_codex(
        &temp,
        &format!(
            "echo 'Codex diagnostic must not reach stdout' >&2\n{}",
            output_file_writer(&json_response(command, "safe"))
        ),
    );

    command_with(&temp, &fake)
        .args([
            "generate",
            "--shell",
            "bash",
            "日本語のファイルを 'quoted' で探して 😀",
        ])
        .assert()
        .success()
        .stdout(format!("{command}\n"))
        .stderr("");
}

#[test]
fn ac4_allowed_review_and_dangerous_risks_preserve_phase1_command_only_output() {
    for risk in ["review", "dangerous"] {
        let temp = TempDir::new().unwrap();
        let fake = fake_codex(
            &temp,
            &output_file_writer(&json_response("printf '%s' ok", risk)),
        );

        command_with(&temp, &fake)
            .args(["generate", "--shell", "bash", "list files"])
            .assert()
            .success()
            .stdout("printf '%s' ok\n")
            .stderr("");
    }
}

#[test]
fn ac6_generate_works_inside_and_outside_a_git_repository() {
    for in_repository in [false, true] {
        let temp = TempDir::new().unwrap();
        let cwd = temp
            .path()
            .join(if in_repository { "inside" } else { "outside" });
        fs::create_dir(&cwd).unwrap();
        if in_repository {
            let status = Command::new("git")
                .args(["init", "-q"])
                .current_dir(&cwd)
                .status()
                .unwrap();
            assert!(status.success());
        }
        let cwd_capture = temp.path().join("cwd");
        let fake = fake_codex(
            &temp,
            &format!(
                "printf '%s' \"$PWD\" > {}\n{}",
                shell_quote(&cwd_capture),
                output_file_writer(&json_response("echo ok", "safe"))
            ),
        );

        command_with(&temp, &fake)
            .current_dir(&cwd)
            .args(["generate", "--shell", "zsh", "list files"])
            .assert()
            .success()
            .stdout("echo ok\n")
            .stderr("");
        assert_eq!(
            fs::read_to_string(cwd_capture).unwrap(),
            fs::canonicalize(&cwd).unwrap().display().to_string()
        );
    }
}

#[test]
fn ac7_model_and_reasoning_are_forwarded_once_without_fallback() {
    let temp = TempDir::new().unwrap();
    let arguments = temp.path().join("arguments");
    let fake = fake_codex(
        &temp,
        &format!(
            "printf '%s\\n' \"$@\" > {}\ncat >/dev/null\necho 'selected model is unavailable' >&2\nexit 1",
            shell_quote(&arguments)
        ),
    );

    command_with(&temp, &fake)
        .args([
            "generate",
            "--shell",
            "fish",
            "--model",
            "unavailable-model",
            "--reasoning-effort",
            "high",
            "list files",
        ])
        .assert()
        .code(ExitCode::ModelUnavailable as i32)
        .stdout("")
        .stderr(predicates::str::contains("unavailable"));

    let arguments = fs::read_to_string(arguments).unwrap();
    assert_eq!(arguments.matches("unavailable-model").count(), 1);
    assert_eq!(
        arguments.matches("model_reasoning_effort=\"high\"").count(),
        1
    );
    assert!(!arguments.contains("gpt-5.6-luna"));
}

#[test]
fn ac8_explicit_shell_is_reflected_in_the_prompt_for_every_supported_shell() {
    for shell in ["bash", "zsh", "fish"] {
        let temp = TempDir::new().unwrap();
        let prompt = temp.path().join("prompt");
        let fake = fake_codex(
            &temp,
            &format!(
                "cat > {}\n{}",
                shell_quote(&prompt),
                output_file_writer(&json_response("echo ok", "safe"))
            ),
        );

        command_with(&temp, &fake)
            .args(["generate", "--shell", shell, "list files"])
            .assert()
            .success()
            .stdout("echo ok\n")
            .stderr("");
        assert!(
            fs::read_to_string(prompt)
                .unwrap()
                .contains(&format!("Target shell: {shell}")),
            "prompt did not retain explicit shell {shell}"
        );
    }
}

#[test]
fn ac4_ac5_cli_maps_success_invalid_output_and_process_errors_to_documented_codes() {
    let cases = [
        (
            output_file_writer(&json_response("echo ok", "safe")),
            ExitCode::Success as i32,
        ),
        (
            output_file_writer(r#"{"command":"echo ok","risk":"safe","extra":true}"#),
            ExitCode::InvalidOutput as i32,
        ),
        (
            output_file_writer(r#"{"command":"echo ok","risk":"unknown"}"#),
            ExitCode::InvalidOutput as i32,
        ),
        (
            "cat >/dev/null\necho 'codex login is required' >&2\nexit 1".to_owned(),
            ExitCode::Unauthenticated as i32,
        ),
        (
            "cat >/dev/null\necho 'unclassified failure' >&2\nexit 1".to_owned(),
            ExitCode::General as i32,
        ),
    ];
    for (body, expected_code) in cases {
        let temp = TempDir::new().unwrap();
        let fake = fake_codex(&temp, &body);
        let assertion = command_with(&temp, &fake)
            .args(["generate", "--shell", "bash", "list files"])
            .assert()
            .code(expected_code);
        if expected_code == ExitCode::Success as i32 {
            assertion.stdout("echo ok\n").stderr("");
        } else {
            assertion.stdout("");
        }
    }

    let temp = TempDir::new().unwrap();
    command_with(&temp, &temp.path().join("missing-codex"))
        .args(["generate", "--shell", "bash", "list files"])
        .assert()
        .code(ExitCode::CodexNotFound as i32)
        .stdout("");

    AssertCommand::cargo_bin("hashai")
        .unwrap()
        .args(["generate", "--shell", "powershell", "list files"])
        .assert()
        .code(ExitCode::ArgumentOrConfig as i32)
        .stdout("");

    let temp = TempDir::new().unwrap();
    let fake = fake_codex(
        &temp,
        &output_file_writer(&json_response("echo ok", "safe")),
    );
    command_with(&temp, &fake)
        .env("HASHAI_TEST_OS", "windows")
        .args(["generate", "--shell", "bash", "list files"])
        .assert()
        .code(ExitCode::UnsupportedPlatform as i32)
        .stdout("");
}

#[test]
fn ac5_timeout_and_cancel_are_cli_e2e_errors() {
    let temp = TempDir::new().unwrap();
    let fake = fake_codex(&temp, "sleep 30");
    command_with(&temp, &fake)
        .args([
            "generate",
            "--shell",
            "bash",
            "--timeout-seconds",
            "1",
            "list files",
        ])
        .assert()
        .code(ExitCode::Timeout as i32)
        .stdout("");

    let marker = temp.path().join("started");
    let fake = fake_codex(&temp, &format!("touch {}\nsleep 30", shell_quote(&marker)));
    let config_home = temp.path().join("cancel-config-home");
    fs::create_dir(&config_home).unwrap();
    let mut child = Command::new(assert_cmd::cargo::cargo_bin("hashai"))
        .env("XDG_CONFIG_HOME", config_home)
        .env("HASHAI_CODEX_BIN", fake)
        .args(["generate", "--shell", "bash", "list files"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_file(&marker, "fake Codex start marker");
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let status = wait_for_child_exit(&mut child, "cancelled hashai process");
    assert_eq!(status.code(), Some(ExitCode::Cancelled as i32));
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}
