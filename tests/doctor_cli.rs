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
    let path = temp.path().join("fake-codex");
    fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn command(temp: &TempDir, codex: &Path) -> Command {
    let mut command = Command::cargo_bin("hashai").unwrap();
    command
        .current_dir(temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .env("HASHAI_CODEX_BIN", codex)
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
    assert_eq!(report["schema_version"], 1);
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
            "integration",
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
