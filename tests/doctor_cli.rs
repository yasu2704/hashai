#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use hashai::ExitCode;
use serde_json::Value;
use tempfile::TempDir;

fn fake_codex(temp: &TempDir, body: &str) -> PathBuf {
    fake_tool(temp, "fake-codex", body)
}

fn fake_tool(temp: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn command(temp: &TempDir, codex: &Path) -> Command {
    let supported_shell = fake_tool(
        temp,
        "doctor-supported-bash",
        "[ \"${1:-}\" = --version ] && { echo 'GNU bash, version 5.2.0'; exit 0; }; exit 1",
    );
    let mut command = Command::cargo_bin("hashai").unwrap();
    command
        .current_dir(temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .env("HASHAI_CODEX_BIN", codex)
        // Generic doctor fixtures must not inherit macOS's unsupported system
        // Bash 3.2. Real-shell coverage explicitly selects and checks a real
        // interpreter instead of using this supported version fixture.
        .env("HASHAI_DOCTOR_BASH_BIN", supported_shell)
        .env("SHELL", "/bin/bash")
        .env_remove("HASHAI_TRIGGER")
        .env_remove("HASHAI_TRIGGER_ENABLED")
        .env_remove("HASHAI_KEYBINDING")
        .env_remove("HASHAI_TIMEOUT_SECONDS")
        .env_remove("HASHAI_SHELL")
        .env_remove("HASHAI_CODEX_MODEL")
        .env_remove("HASHAI_CODEX_REASONING_EFFORT");
    command
}

fn capable_fake_body() -> &'static str {
    r#"
case "${1:-}" in
  --version) printf '%s\n' 'codex 9.9.9'; exit 0 ;;
  exec)
    if [ "${2:-}" = "--help" ]; then
      printf '%s\n' 'Usage: codex exec --ephemeral --ignore-user-config --ignore-rules --model --config project_doc_max_bytes=0 project_doc_fallback_filenames=[] --sandbox --disable shell_tool browser_use computer_use apps --skip-git-repo-check --output-schema --output-last-message'
      exit 0
    fi
    out=''
    while [ "$#" -gt 0 ]; do
      case "$1" in --output-last-message) out="$2"; shift 2;; *) shift;; esac
    done
    cat >/dev/null
    printf '%s' '{"command":"printf ok","risk":"safe"}' > "$out"
    exit 0 ;;
  login)
    [ "${2:-}" = --help ] && { printf '%s\n' status; exit 0; }
    [ "${2:-}" = status ] && { printf '%s\n' 'logged in'; exit 0; } ;;
esac
exit 1
"#
}

#[test]
fn ac1_ac2_ac3_ac4_ac5_ac7_static_json_is_ordered_and_redacted() {
    let temp = TempDir::new().unwrap();
    let fake = fake_codex(&temp, capable_fake_body());
    let output = command(&temp, &fake)
        .args(["doctor", "--format", "json", "--shell", "bash"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 2);
    assert_eq!(report["overall"], "WARN");
    assert_eq!(report["exit"], 0);
    let ids: Vec<_> = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|check| check["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
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
            "live_probe"
        ]
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!text.contains(&temp.path().display().to_string()));
    assert!(!text.contains("HASHAI_CODEX_BIN"));
}

#[test]
fn ac6_exit_precedence_uses_unsupported_before_missing_codex() {
    let temp = TempDir::new().unwrap();
    let missing = temp.path().join("not-present");
    command(&temp, &missing)
        .env("HASHAI_TEST_OS", "windows")
        .args(["doctor", "--format", "json", "--shell", "bash"])
        .assert()
        .code(ExitCode::UnsupportedPlatform as i32);

    command(&temp, &missing)
        .args(["doctor", "--format", "json", "--shell", "powershell"])
        .assert()
        .code(ExitCode::UnsupportedPlatform as i32);
}

#[test]
fn ac8_ac9_live_uses_one_isolated_probe_without_fallback() {
    let temp = TempDir::new().unwrap();
    let count = temp.path().join("probe-count");
    let fake = fake_codex(
        &temp,
        &format!(
            r#"case "${{1:-}}" in
  --version) printf '%s\n' 'codex 9.9.9'; exit 0 ;;
  login) [ "${{2:-}}" = --help ] && {{ printf '%s\n' status; exit 0; }}; [ "${{2:-}}" = status ] && exit 0 ;;
  exec)
    if [ "${{2:-}}" = --help ]; then printf '%s\n' 'Usage: codex exec --ephemeral --ignore-user-config --ignore-rules --model --config project_doc_max_bytes=0 project_doc_fallback_filenames=[] --sandbox --disable shell_tool browser_use computer_use apps --skip-git-repo-check --output-schema --output-last-message'; exit 0; fi
    n=0; [ -f '{}' ] && n=$(cat '{}'); printf '%s' $((n+1)) > '{}'
    out=''; while [ "$#" -gt 0 ]; do case "$1" in --output-last-message) out="$2"; shift 2;; *) shift;; esac; done
    cat >/dev/null; printf '%s' '{{"command":"printf ok","risk":"safe"}}' > "$out"; exit 0 ;;
esac
exit 1"#,
            count.display(),
            count.display(),
            count.display()
        ),
    );
    command(&temp, &fake)
        .args(["doctor", "--live", "--format", "json", "--shell", "bash"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(count).unwrap(), "1");
}

#[test]
fn ac9_fake_capability_version_auth_and_unknown_failures_map_to_machine_exit_codes() {
    let temp = TempDir::new().unwrap();
    let missing_feature = fake_codex(
        &temp,
        capable_fake_body().replace(" --output-schema", "").as_str(),
    );
    command(&temp, &missing_feature)
        .args(["doctor", "--format", "json", "--shell", "bash"])
        .assert()
        .code(ExitCode::General as i32);

    let missing_version = fake_codex(&temp, "exit 1");
    command(&temp, &missing_version)
        .args(["doctor", "--format", "json", "--shell", "bash"])
        .assert()
        .code(ExitCode::General as i32);

    for (stderr, expected) in [
        ("Please run codex login", ExitCode::Unauthenticated),
        ("opaque failure", ExitCode::General),
    ] {
        let fake = fake_codex(
            &temp,
            &format!(
                r#"case "${{1:-}}" in
  --version) echo codex; exit 0 ;;
  login) [ "${{2:-}}" = --help ] && {{ echo status; exit 0; }}; [ "${{2:-}}" = status ] && exit 0 ;;
  exec) if [ "${{2:-}}" = --help ]; then echo 'Usage: codex exec --ephemeral --ignore-user-config --ignore-rules --model --config project_doc_max_bytes=0 project_doc_fallback_filenames=[] --sandbox shell_tool browser_use computer_use apps --skip-git-repo-check --output-schema --output-last-message'; exit 0; fi; echo '{}' >&2; exit 1 ;;
esac
exit 1"#,
                stderr
            ),
        );
        command(&temp, &fake)
            .args(["doctor", "--live", "--format", "json", "--shell", "bash"])
            .assert()
            .code(expected as i32);
    }
}

#[test]
fn static_auth_classifies_logged_out_text_from_stdout_and_stderr_even_when_status_is_nonzero() {
    let temp = TempDir::new().unwrap();
    for message in ["logged out", "not logged in", "unauthenticated"] {
        for stream in ["stdout", "stderr"] {
            let redirect = if stream == "stderr" { " >&2" } else { "" };
            let fake = fake_codex(
                &temp,
                &format!(
                    r#"case "${{1:-}}" in
 --version) echo 'codex 9.9.9' ;;
 exec) [ "${{2:-}}" = --help ] && echo 'exec --ephemeral --ignore-user-config --ignore-rules --model --config --sandbox --skip-git-repo-check --output-schema --output-last-message' ;;
 login) [ "${{2:-}}" = --help ] && {{ echo status; exit 0; }}; echo '{message}'{redirect}; exit 1 ;;
esac"#
                ),
            );
            let output = command(&temp, &fake)
                .args(["doctor", "--format", "json", "--shell", "bash"])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(4), "{message} from {stream}");
            let report: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(report["checks"][21]["status"], "FAIL");
            assert_eq!(report["checks"][21]["exit"], 4);
        }
    }
}

#[test]
fn every_static_capability_is_individually_reported_when_missing() {
    let temp = TempDir::new().unwrap();
    let capabilities = [
        "exec",
        "--ephemeral",
        "--ignore-user-config",
        "--ignore-rules",
        "--model",
        "--config",
        "--sandbox",
        "--skip-git-repo-check",
        "--output-schema",
        "--output-last-message",
    ];
    for missing in capabilities {
        let help = capabilities
            .iter()
            .copied()
            .filter(|token| *token != missing)
            .collect::<Vec<_>>()
            .join(" ");
        let fake = fake_codex(
            &temp,
            &format!(
                r#"case "${{1:-}}" in
 --version) echo 'codex 9.9.9' ;;
 exec) [ "${{2:-}}" = --help ] && {{ echo '{help}'; exit 0; }} ;;
 login) [ "${{2:-}}" = --help ] && {{ echo status; exit 0; }}; echo 'logged in' ;;
esac"#
            ),
        );
        let output = command(&temp, &fake)
            .args(["doctor", "--format", "json", "--shell", "bash"])
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let failed = report["checks"].as_array().unwrap().iter().any(|check| {
            check["status"] == "FAIL" && check["message"] == "required Codex capability missing"
        });
        assert!(failed, "missing {missing} was not individually detected");
    }
}

#[test]
fn malformed_codex_version_is_not_accepted() {
    let temp = TempDir::new().unwrap();
    let fake = fake_codex(
        &temp,
        "[ \"${1:-}\" = --version ] && { echo codex-version-unknown; exit 0; }; exit 1",
    );
    command(&temp, &fake)
        .args(["doctor", "--format", "json", "--shell", "bash"])
        .assert()
        .code(ExitCode::General as i32);
}

#[test]
fn live_probe_uses_exact_isolated_argv_model_reasoning_and_requested_nonrepo_cwd() {
    let temp = TempDir::new().unwrap();
    let record = temp.path().join("argv");
    let fake = fake_codex(
        &temp,
        &format!(
            r#"case "${{1:-}}" in
 --version) echo 'codex 9.9.9' ;;
 login) [ "${{2:-}}" = --help ] && {{ echo status; exit 0; }}; echo 'logged in' ;;
 exec)
  if [ "${{2:-}}" = --help ]; then echo 'exec --ephemeral --ignore-user-config --ignore-rules --model --config --sandbox --disable --skip-git-repo-check --output-schema --output-last-message'; exit 0; fi
  printf '%s\n' "$PWD" > '{}'; printf '%s\n' "$@" >> '{}'
  out=''; while [ "$#" -gt 0 ]; do case "$1" in --output-last-message) out="$2"; shift 2;; *) shift;; esac; done
  cat >/dev/null; printf '%s' '{{"command":"printf ok","risk":"safe"}}' > "$out" ;;
esac"#,
            record.display(),
            record.display()
        ),
    );
    let outside = temp.path().join("outside-no-git");
    fs::create_dir(&outside).unwrap();
    let output = command(&temp, &fake)
        .current_dir(&outside)
        .env("HASHAI_CODEX_MODEL", "model-canary")
        .env("HASHAI_CODEX_REASONING_EFFORT", "reasoning-canary")
        .args(["doctor", "--live", "--format", "json", "--shell", "bash"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let recorded = fs::read_to_string(record).unwrap();
    let mut recorded = recorded.lines();
    let recorded_cwd = PathBuf::from(recorded.next().expect("recorded Codex cwd"));
    assert_eq!(
        recorded_cwd.canonicalize().unwrap(),
        outside.canonicalize().unwrap(),
        "Codex must run in the requested non-repository directory"
    );
    let argv: Vec<_> = recorded.collect();
    assert_eq!(argv.first(), Some(&"exec"));
    for required in [
        "--ephemeral",
        "--ignore-user-config",
        "--ignore-rules",
        "--skip-git-repo-check",
        "read-only",
        "shell_tool",
        "browser_use",
        "computer_use",
        "apps",
        "model-canary",
        "model_reasoning_effort=\"reasoning-canary\"",
        "project_doc_max_bytes=0",
        "project_doc_fallback_filenames=[]",
    ] {
        assert!(argv.contains(&required), "missing {required} in {argv:?}");
    }
    assert_eq!(
        argv.iter()
            .filter(|argument| **argument == "--output-last-message")
            .count(),
        1
    );
}

#[test]
fn human_and_json_reports_redact_tool_output_path_and_secret_canaries() {
    let temp = TempDir::new().unwrap();
    let secret = "tok_live_DOCTOR_REDACTION_CANARY";
    let fake = fake_codex(
        &temp,
        &format!(
            r#"case "${{1:-}}" in
 --version) echo 'codex 9.9.9 {secret} $PWD' ;;
 exec) [ "${{2:-}}" = --help ] && echo 'exec --ephemeral --ignore-user-config --ignore-rules --model --config --sandbox --skip-git-repo-check --output-schema --output-last-message' ;;
 login) [ "${{2:-}}" = --help ] && {{ echo status; exit 0; }}; echo 'logged in {secret}' ;;
esac"#
        ),
    );
    for format in ["human", "json"] {
        let output = command(&temp, &fake)
            .args(["doctor", "--format", format, "--shell", "bash"])
            .output()
            .unwrap();
        let rendered = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!rendered.contains(secret));
        assert!(!rendered.contains(&temp.path().display().to_string()));
        if format == "json" {
            let report: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(serde_json::to_value(&report).unwrap(), report);
        }
    }
}

#[test]
fn integration_mismatch_and_unreadable_artifact_are_distinct_machine_checks() {
    let temp = TempDir::new().unwrap();
    let fake = fake_codex(&temp, capable_fake_body());
    command(&temp, &fake)
        .args(["integration", "install", "--shell", "bash"])
        .assert()
        .success();
    let artifact = temp.path().join("data/hashai/integrations/hashai.bash");
    fs::write(&artifact, "# hashai-integration-version: obsolete\n").unwrap();
    let mismatch = command(&temp, &fake)
        .args(["doctor", "--format", "json", "--shell", "bash"])
        .output()
        .unwrap();
    let mismatch: Value = serde_json::from_slice(&mismatch.stdout).unwrap();
    assert_eq!(mismatch["checks"][24]["status"], "WARN");
    fs::remove_file(&artifact).unwrap();
    fs::create_dir(&artifact).unwrap();
    let unreadable = command(&temp, &fake)
        .args(["doctor", "--format", "json", "--shell", "bash"])
        .output()
        .unwrap();
    assert_eq!(unreadable.status.code(), Some(ExitCode::General as i32));
    let unreadable: Value = serde_json::from_slice(&unreadable.stdout).unwrap();
    assert_eq!(unreadable["checks"][24]["status"], "FAIL");
}

#[test]
fn live_keybinding_harness_loads_the_current_artifact_and_configured_key() {
    let temp = TempDir::new().unwrap();
    let fake = fake_codex(&temp, capable_fake_body());
    command(&temp, &fake)
        .args([
            "integration",
            "install",
            "--shell",
            "bash",
            "--keybinding",
            "ctrl-x",
        ])
        .assert()
        .success();
    let shell = fake_tool(
        &temp,
        "fake-bash",
        r#"if [ "${1:-}" = --version ]; then echo 'GNU bash, version 5.2.0'; exit 0; fi
case "$*" in *hashai.bash*'\C-x'*) printf '%s\n' 'HASHAI_PRE:unbound' 'HASHAI_POST:owner' 'HASHAI_UNCHANGED:no'; exit 0;; *) exit 1;; esac"#,
    );
    let output = command(&temp, &fake)
        .env("HASHAI_DOCTOR_BASH_BIN", &shell)
        .env("HASHAI_KEYBINDING", "ctrl-x")
        .args(["doctor", "--live", "--format", "json", "--shell", "bash"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["checks"][23]["status"], "PASS");
}

#[test]
fn live_keybinding_harness_uses_shell_specific_artifact_key_and_keymaps() {
    let temp = TempDir::new().unwrap();
    let codex = fake_codex(&temp, capable_fake_body());
    for (shell_name, version, key, env) in [
        (
            "bash",
            "GNU bash, version 5.2.0",
            r#"\C-x"#,
            "HASHAI_DOCTOR_BASH_BIN",
        ),
        ("zsh", "zsh 5.9", "^X", "HASHAI_DOCTOR_ZSH_BIN"),
        (
            "fish",
            "fish, version 3.6.0",
            "ctrl-x",
            "HASHAI_DOCTOR_FISH_BIN",
        ),
    ] {
        command(&temp, &codex)
            .args([
                "integration",
                "install",
                "--shell",
                shell_name,
                "--keybinding",
                "ctrl-x",
            ])
            .assert()
            .success();
        let shell = fake_tool(
            &temp,
            &format!("fake-{shell_name}"),
            &format!(
                r#"if [ "${{1:-}}" = --version ]; then echo '{version}'; exit 0; fi
case "$*" in *hashai.{shell_name}*'{key}'*) printf '%s\n' 'HASHAI_PRE:unbound,unbound' 'HASHAI_POST:owner,owner' 'HASHAI_UNCHANGED:no'; exit 0;; *) exit 1;; esac"#
            ),
        );
        let output = command(&temp, &codex)
            .env(env, shell)
            .env("HASHAI_KEYBINDING", "ctrl-x")
            .args([
                "doctor", "--live", "--format", "json", "--shell", shell_name,
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{shell_name}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report["checks"][23]["status"], "PASS",
            "{shell_name}: {report}"
        );
    }
}

#[test]
fn live_probe_reports_typed_success_unauthenticated_and_model_unavailable_states() {
    let temp = TempDir::new().unwrap();
    let success = fake_codex(&temp, capable_fake_body());
    let success_report: Value = serde_json::from_slice(
        &command(&temp, &success)
            .args(["doctor", "--live", "--format", "json", "--shell", "bash"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(success_report["checks"][21]["status"], "PASS");
    assert_eq!(success_report["checks"][22]["status"], "PASS");
    assert_eq!(success_report["checks"][29]["status"], "PASS");

    for (stderr, auth, model, probe, exit) in [
        (
            "not logged in",
            "FAIL",
            "WARN",
            "FAIL",
            ExitCode::Unauthenticated as i32,
        ),
        (
            "model unavailable",
            "PASS",
            "FAIL",
            "FAIL",
            ExitCode::ModelUnavailable as i32,
        ),
    ] {
        let fake = fake_codex(
            &temp,
            &format!(
                r#"case "${{1:-}}" in
 --version) echo 'codex 9.9.9' ;;
 login) [ "${{2:-}}" = --help ] && {{ echo status; exit 0; }}; echo 'logged in' ;;
 exec) if [ "${{2:-}}" = --help ]; then echo 'exec --ephemeral --ignore-user-config --ignore-rules --model --config --sandbox --disable --skip-git-repo-check --output-schema --output-last-message'; else echo '{stderr}' >&2; exit 1; fi ;;
esac"#
            ),
        );
        let output = command(&temp, &fake)
            .args(["doctor", "--live", "--format", "json", "--shell", "bash"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(exit));
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["checks"][21]["status"], auth);
        assert_eq!(report["checks"][22]["status"], model);
        assert_eq!(report["checks"][29]["status"], probe);
    }
}

#[test]
fn live_keybinding_harness_has_real_shell_semantics_when_shells_are_available() {
    let temp = TempDir::new().unwrap();
    let codex = fake_codex(&temp, capable_fake_body());
    let (shell_name, program, shell_env) = if cfg!(target_os = "macos") {
        ("zsh", Path::new("/bin/zsh"), "HASHAI_DOCTOR_ZSH_BIN")
    } else {
        ("bash", Path::new("/bin/bash"), "HASHAI_DOCTOR_BASH_BIN")
    };
    assert!(
        program.is_file(),
        "required real {shell_name} is unavailable"
    );
    let version_ok = if shell_name == "zsh" {
        std::process::Command::new(program)
            .args(["-fc", "autoload -Uz is-at-least; is-at-least 5.8"])
            .status()
            .unwrap()
            .success()
    } else {
        std::process::Command::new(program)
            .args(["-c", "test \"${BASH_VERSINFO[0]}\" -gt 5 || { test \"${BASH_VERSINFO[0]}\" -eq 5 && test \"${BASH_VERSINFO[1]}\" -ge 2; }"])
            .status()
            .unwrap()
            .success()
    };
    assert!(
        version_ok,
        "real {shell_name} is below the production minimum"
    );
    command(&temp, &codex)
        .args([
            "integration",
            "install",
            "--shell",
            shell_name,
            "--keybinding",
            "ctrl-x",
        ])
        .assert()
        .success();
    let output = command(&temp, &codex)
        .env("HOME", temp.path().join("empty-home"))
        .env("HASHAI_KEYBINDING", "ctrl-x")
        .env(shell_env, program)
        .args([
            "doctor", "--live", "--format", "json", "--shell", shell_name,
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{shell_name}");
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["checks"][23]["status"], "PASS",
        "{shell_name}: {report}"
    );
}

#[test]
fn disabled_live_keybinding_accepts_an_unchanged_foreign_binding() {
    let temp = TempDir::new().unwrap();
    let codex = fake_codex(&temp, capable_fake_body());
    command(&temp, &codex)
        .args([
            "integration",
            "install",
            "--shell",
            "bash",
            "--disable-trigger",
        ])
        .assert()
        .success();
    let shell = fake_tool(
        &temp,
        "disabled-bash",
        r#"if [ "${1:-}" = --version ]; then echo 'GNU bash, version 5.2.0'; exit 0; fi
printf '%s\n' 'HASHAI_PRE:foreign' 'HASHAI_POST:foreign' 'HASHAI_UNCHANGED:yes'"#,
    );
    let output = command(&temp, &codex)
        .env("HASHAI_DOCTOR_BASH_BIN", shell)
        .env("HASHAI_TRIGGER_ENABLED", "false")
        .args(["doctor", "--live", "--format", "json", "--shell", "bash"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["checks"][23]["status"], "PASS");
}

#[test]
fn doctor_v2_reports_tracked_bash_as_current_with_manual_startup() {
    let temp = TempDir::new().unwrap();
    let fake = fake_codex(&temp, capable_fake_body());
    command(&temp, &fake)
        .args(["integration", "install", "bash"])
        .assert()
        .success();
    let output = command(&temp, &fake)
        .args(["doctor", "--format", "json", "--shell", "bash"])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["checks"][24]["message"], "current");
    assert_eq!(report["checks"][24]["status"], "PASS");
    assert_eq!(report["checks"][25]["message"], "manual-startup");
    assert_eq!(report["checks"][26]["message"], "manual-startup");
}

#[test]
fn doctor_v2_loader_unsafe_is_fail_even_when_artifact_is_unsafe() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let fake = fake_codex(&temp, capable_fake_body());
    let fish = fake_tool(
        &temp,
        "doctor-supported-fish",
        "[ \"${1:-}\" = --version ] && { echo 'fish, version 4.0.0'; exit 0; }; exit 1",
    );
    command(&temp, &fake)
        .args(["integration", "install", "fish"])
        .assert()
        .success();
    let artifact = temp.path().join("data/hashai/integrations/hashai.fish");
    fs::remove_file(&artifact).unwrap();
    fs::create_dir(&artifact).unwrap();
    let loader = temp.path().join("config/fish/conf.d/hashai.fish");
    fs::remove_file(&loader).unwrap();
    symlink(temp.path().join("outside"), &loader).unwrap();
    let output = command(&temp, &fake)
        .env("HASHAI_DOCTOR_FISH_BIN", fish)
        .args(["doctor", "--format", "json", "--shell", "fish"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["checks"][24]["status"], "FAIL");
    assert_eq!(report["checks"][25]["status"], "FAIL");
    assert_eq!(
        report["checks"][25]["message"],
        "loader-unsafe-or-unreadable"
    );
    assert_eq!(report["checks"][26]["status"], "WARN");
}

#[test]
fn doctor_v2_distinguishes_tracked_prior_and_interrupted_recoverable() {
    let temp = TempDir::new().unwrap();
    let fake = fake_codex(&temp, capable_fake_body());
    command(&temp, &fake)
        .args(["integration", "install", "bash"])
        .assert()
        .success();
    let prior = command(&temp, &fake)
        .env("HASHAI_KEYBINDING", "ctrl-x")
        .args(["doctor", "--format", "json", "--shell", "bash"])
        .output()
        .unwrap();
    let prior: Value = serde_json::from_slice(&prior.stdout).unwrap();
    assert_eq!(prior["checks"][24]["status"], "WARN");
    assert_eq!(prior["checks"][24]["message"], "prior-supported");

    command(&temp, &fake)
        .env("HASHAI_TEST_INTEGRATION_FAULT_PHASE", "artifact-published")
        .args(["integration", "update", "--keybinding", "ctrl-x"])
        .assert()
        .code(1);
    let interrupted = command(&temp, &fake)
        .env("HASHAI_KEYBINDING", "ctrl-x")
        .args(["doctor", "--format", "json", "--shell", "bash"])
        .output()
        .unwrap();
    let interrupted: Value = serde_json::from_slice(&interrupted.stdout).unwrap();
    assert_eq!(interrupted["checks"][24]["status"], "WARN");
    assert_eq!(
        interrupted["checks"][24]["message"],
        "interrupted-recoverable"
    );
}
