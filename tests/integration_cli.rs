#![cfg(unix)]

use std::{fs, path::Path};

use assert_cmd::Command;
use hashai::integration::ARTIFACT_VERSION;
use predicates::prelude::PredicateBooleanExt;

#[test]
fn ac1_ac2_ac6_cli_generates_lists_and_repeats_managed_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let data_home = temp.path().join("data");
    let expected = data_home.join("hashai/integrations/hashai.bash");

    command(&data_home)
        .args(["integration", "generate", "--shell", "bash"])
        .assert()
        .success()
        .stdout(format!("bash\twritten\t{}\n", expected.display()));
    let bytes_after_generate = fs::read(&expected).unwrap();

    command(&data_home)
        .args(["integration", "list"])
        .assert()
        .success()
        .stdout(format!(
            "bash\t{ARTIFACT_VERSION}\tcurrent\t{}\n",
            expected.display()
        ));
    assert_eq!(fs::read(&expected).unwrap(), bytes_after_generate);

    command(&data_home)
        .args(["integration", "generate", "--shell", "bash"])
        .assert()
        .success()
        .stdout(format!("bash\tunchanged\t{}\n", expected.display()));
}

#[test]
fn ac3_ac4_ac8_cli_update_creates_a_backup_and_rejects_unknown_shells() {
    let temp = tempfile::tempdir().unwrap();
    let data_home = temp.path().join("data");
    let artifact = data_home.join("hashai/integrations/hashai.zsh");
    let backup = data_home.join("hashai/integrations/hashai.zsh.bak");
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    let old = b"# hashai-integration-version: obsolete\nold\n";
    fs::write(&artifact, old).unwrap();

    command(&data_home)
        .args(["integration", "update"])
        .assert()
        .success()
        .stdout(format!("zsh\twritten\t{}\n", artifact.display()));
    assert_eq!(fs::read(backup).unwrap(), old);

    command(&data_home)
        .args(["integration", "generate", "--shell", "auto"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains(
            "integration shell must be bash, zsh, or fish",
        ));
}

#[test]
fn ac1_generate_keeps_the_documented_positional_shell_form() {
    let temp = tempfile::tempdir().unwrap();
    let data_home = temp.path().join("data");
    let expected = data_home.join("hashai/integrations/hashai.fish");

    command(&data_home)
        .args(["integration", "generate", "fish"])
        .assert()
        .success()
        .stdout(format!("fish\twritten\t{}\n", expected.display()));
}

#[test]
fn ac7_cli_ignores_relative_xdg_data_home() {
    let temp = tempfile::tempdir().unwrap();
    Command::cargo_bin("hashai")
        .unwrap()
        .current_dir(temp.path())
        .env("XDG_DATA_HOME", "relative-data-home")
        .args(["integration", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("relative-data-home").not());
}

#[test]
fn ac4_ac8_cli_update_reports_a_failed_shell_continues_and_rerun_converges() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let data_home = temp.path().join("data");
    let directory = data_home.join("hashai/integrations");
    fs::create_dir_all(&directory).unwrap();
    for shell in ["bash", "zsh", "fish"] {
        fs::write(
            directory.join(format!("hashai.{shell}")),
            "# hashai-integration-version: obsolete\n",
        )
        .unwrap();
    }
    let zsh_backup = directory.join("hashai.zsh.bak");
    symlink(temp.path().join("outside"), &zsh_backup).unwrap();

    command(&data_home)
        .args(["integration", "update"])
        .assert()
        .code(1)
        .stdout(format!(
            "bash\twritten\t{}\nfish\twritten\t{}\n",
            directory.join("hashai.bash").display(),
            directory.join("hashai.fish").display()
        ))
        .stderr(predicates::str::contains(
            "integration update for zsh failed",
        ));
    assert_eq!(
        fs::read_to_string(directory.join("hashai.zsh")).unwrap(),
        "# hashai-integration-version: obsolete\n"
    );

    fs::remove_file(zsh_backup).unwrap();
    command(&data_home)
        .args(["integration", "update"])
        .assert()
        .success()
        .stdout(format!(
            "bash\tunchanged\t{}\nzsh\twritten\t{}\nfish\tunchanged\t{}\n",
            directory.join("hashai.bash").display(),
            directory.join("hashai.zsh").display(),
            directory.join("hashai.fish").display()
        ));
}

fn command(data_home: &Path) -> Command {
    let mut command = Command::cargo_bin("hashai").unwrap();
    command
        .env("XDG_DATA_HOME", data_home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HASHAI_TRIGGER")
        .env_remove("HASHAI_TIMEOUT_SECONDS")
        .env_remove("HASHAI_SHELL")
        .env_remove("HASHAI_CODEX_MODEL")
        .env_remove("HASHAI_CODEX_REASONING_EFFORT");
    command
}
