#![cfg(unix)]

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use std::{fs, path::Path};

fn command(data_home: &Path) -> Command {
    let data_home = data_home
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(data_home.file_name().unwrap());
    let mut command = Command::cargo_bin("hashai").unwrap();
    command
        .env("HOME", data_home.join("home"))
        .env("XDG_DATA_HOME", &data_home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HASHAI_TRIGGER")
        .env_remove("HASHAI_TRIGGER_ENABLED")
        .env_remove("HASHAI_KEYBINDING");
    command
}

#[test]
fn install_list_update_and_uninstall_are_manifest_backed() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    command(&data)
        .args(["integration", "install", "bash"])
        .assert()
        .success();
    command(&data)
        .args(["integration", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("bash\t"));
    command(&data)
        .args(["integration", "update"])
        .assert()
        .success();
    command(&data)
        .args(["integration", "uninstall", "bash"])
        .assert()
        .success();
    assert!(!data.join("hashai/integrations/hashai.bash").exists());
}

#[test]
fn strict_update_never_overwrites_an_untracked_foreign_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let artifact = data.join("hashai/integrations/hashai.zsh");
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    let foreign = b"foreign bytes\n";
    fs::write(&artifact, foreign).unwrap();
    command(&data)
        .args(["integration", "update"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains(
            "delete it manually and reinstall",
        ));
    assert_eq!(fs::read(artifact).unwrap(), foreign);
}

#[test]
fn relative_xdg_data_home_is_ignored() {
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
fn tracked_update_continues_other_shells_and_rerun_converges() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    for shell in ["bash", "zsh"] {
        command(&data)
            .args(["integration", "install", shell])
            .assert()
            .success();
    }
    let root = data.join("hashai/integrations");
    let bash = root.join("hashai.bash");
    let zsh = root.join("hashai.zsh");
    let original_zsh = fs::read(&zsh).unwrap();
    fs::write(&zsh, "modified\n").unwrap();
    command(&data)
        .args(["integration", "update", "--keybinding", "ctrl-x"])
        .assert()
        .code(1);
    assert!(
        fs::read_to_string(&bash)
            .unwrap()
            .contains("keybinding='\\C-x'")
    );
    assert_eq!(fs::read_to_string(&zsh).unwrap(), "modified\n");

    fs::write(&zsh, original_zsh).unwrap();
    command(&data)
        .args(["integration", "update", "--keybinding", "ctrl-x"])
        .assert()
        .success();
    assert!(fs::read_to_string(zsh).unwrap().contains("keybinding='^X'"));
}
