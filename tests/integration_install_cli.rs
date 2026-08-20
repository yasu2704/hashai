use std::fs;

use assert_cmd::Command;
use fs2::FileExt;

fn command(temp: &tempfile::TempDir) -> Command {
    let root = temp.path().canonicalize().unwrap();
    let mut command = Command::cargo_bin("hashai").unwrap();
    command
        .env("HOME", root.join("home"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"));
    command
}

#[test]
fn ac1_generate_is_unknown_and_does_not_write() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "generate", "fish"])
        .assert()
        .code(2);
    assert!(!temp.path().join("data/hashai/integrations").exists());
    assert!(!temp.path().join("config/fish/conf.d/hashai.fish").exists());
}

#[test]
fn ac2_fish_install_and_uninstall_manage_loader_artifact_and_manifest() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "fish"])
        .assert()
        .success();
    let root = temp.path().join("data/hashai/integrations");
    assert!(root.join("hashai.fish").is_file());
    assert!(root.join("fish.manifest.json").is_file());
    let loader = temp.path().join("config/fish/conf.d/hashai.fish");
    assert!(loader.is_file());
    assert!(fs::read_to_string(&loader).unwrap().contains("source"));

    command(&temp)
        .args(["integration", "uninstall", "--shell", "fish"])
        .assert()
        .success();
    assert!(!root.join("hashai.fish").exists());
    assert!(!root.join("fish.manifest.json").exists());
    assert!(!loader.exists());
}

#[test]
fn ac3_bash_is_manual_and_both_shell_forms_conflict() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "bash\tstartup\tmanual-action-required",
        ))
        .stdout(predicates::str::contains("hashai snippet begin"));
    assert!(!temp.path().join("home/.bashrc").exists());

    command(&temp)
        .args(["integration", "install", "bash", "--shell", "bash"])
        .assert()
        .code(2);
}

#[test]
fn ac2_fish_auto_to_manual_removes_only_the_tracked_loader() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "fish"])
        .assert()
        .success();
    let loader = temp.path().join("config/fish/conf.d/hashai.fish");
    assert!(loader.is_file());

    command(&temp)
        .args(["integration", "install", "fish", "--manual"])
        .assert()
        .success();
    assert!(!loader.exists());
    let manifest = fs::read_to_string(
        temp.path()
            .join("data/hashai/integrations/fish.manifest.json"),
    )
    .unwrap();
    assert!(manifest.contains("\"desired_mode\": \"manual\""));
    assert!(manifest.contains("\"loader\": null"));
}

#[test]
fn ac1_interrupted_install_resumes_from_exact_journal_state() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .env("HASHAI_TEST_INTEGRATION_FAULT_PHASE", "artifact-published")
        .args(["integration", "install", "fish"])
        .assert()
        .code(1);
    let root = temp.path().join("data/hashai/integrations");
    assert!(root.join("fish.journal.json").is_file());
    assert!(root.join("hashai.fish").is_file());

    command(&temp)
        .args(["integration", "install", "fish"])
        .assert()
        .success();
    assert!(!root.join("fish.journal.json").exists());
    assert!(root.join("fish.manifest.json").is_file());
    assert!(temp.path().join("config/fish/conf.d/hashai.fish").is_file());
}

#[test]
fn ac5_interrupted_uninstall_resumes_after_loader_removal() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "fish"])
        .assert()
        .success();
    command(&temp)
        .env(
            "HASHAI_TEST_INTEGRATION_FAULT_PHASE",
            "loader-removed-or-not-applicable",
        )
        .args(["integration", "uninstall", "fish"])
        .assert()
        .code(1);
    let root = temp.path().join("data/hashai/integrations");
    assert!(root.join("fish.journal.json").is_file());
    assert!(root.join("hashai.fish").is_file());
    assert!(!temp.path().join("config/fish/conf.d/hashai.fish").exists());

    command(&temp)
        .args(["integration", "uninstall", "fish"])
        .assert()
        .success();
    assert!(!root.join("fish.journal.json").exists());
    assert!(!root.join("hashai.fish").exists());
    assert!(!root.join("fish.manifest.json").exists());
}

#[test]
fn ac2_foreign_fish_loader_blocks_install_before_any_write() {
    let temp = tempfile::tempdir().unwrap();
    let loader = temp.path().join("config/fish/conf.d/hashai.fish");
    fs::create_dir_all(loader.parent().unwrap()).unwrap();
    fs::write(&loader, "foreign loader\n").unwrap();

    command(&temp)
        .args(["integration", "install", "fish"])
        .assert()
        .code(1);
    assert_eq!(fs::read_to_string(loader).unwrap(), "foreign loader\n");
    assert!(
        !temp
            .path()
            .join("data/hashai/integrations/hashai.fish")
            .exists()
    );
    assert!(
        !temp
            .path()
            .join("data/hashai/integrations/fish.manifest.json")
            .exists()
    );
}

#[test]
fn ac1_install_recovery_converges_at_every_published_phase() {
    for phase in [
        "artifact-published",
        "loader-published-or-not-applicable",
        "manifest-published",
    ] {
        let temp = tempfile::tempdir().unwrap();
        command(&temp)
            .env("HASHAI_TEST_INTEGRATION_FAULT_PHASE", phase)
            .args(["integration", "install", "fish"])
            .assert()
            .code(1);
        command(&temp)
            .args(["integration", "install", "fish"])
            .assert()
            .success();
        let root = temp.path().join("data/hashai/integrations");
        assert!(!root.join("fish.journal.json").exists(), "{phase}");
        assert!(root.join("fish.manifest.json").is_file(), "{phase}");
        assert!(
            temp.path().join("config/fish/conf.d/hashai.fish").is_file(),
            "{phase}"
        );
    }
}

#[test]
fn ac4_environment_paths_are_lexically_normalized_before_render_and_write() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("outer/../data");
    let config = temp.path().join("cfg/./nested/..");
    let mut invocation = Command::cargo_bin("hashai").unwrap();
    invocation
        .env("HOME", temp.path().join("home"))
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["integration", "install", "fish"])
        .assert()
        .success();
    let artifact = temp.path().join("data/hashai/integrations/hashai.fish");
    let loader = temp.path().join("cfg/fish/conf.d/hashai.fish");
    assert!(artifact.is_file());
    assert!(loader.is_file());
    let rendered = fs::read_to_string(loader).unwrap();
    assert!(rendered.contains(artifact.to_str().unwrap()));
    assert!(!rendered.contains("/../"));
    assert!(!rendered.contains("/./"));
}

#[test]
fn ac4_symlink_ancestor_is_rejected_without_writing_outside() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let config_link = temp.path().join("config-link");
    symlink(&outside, &config_link).unwrap();
    let mut invocation = Command::cargo_bin("hashai").unwrap();
    invocation
        .env("HOME", temp.path().join("home"))
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .env("XDG_CONFIG_HOME", &config_link)
        .args(["integration", "install", "fish"])
        .assert()
        .code(1);
    assert!(!outside.join("fish/conf.d/hashai.fish").exists());
    assert!(
        !temp
            .path()
            .join("data/hashai/integrations/hashai.fish")
            .exists()
    );
}

#[test]
fn ac1_tracked_update_uses_update_journal_and_recovers() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .success();
    command(&temp)
        .env("HASHAI_TEST_INTEGRATION_FAULT_PHASE", "artifact-published")
        .args(["integration", "update", "--keybinding", "ctrl-x"])
        .assert()
        .code(1);
    let journal = temp
        .path()
        .join("data/hashai/integrations/bash.journal.json");
    assert!(
        fs::read_to_string(&journal)
            .unwrap()
            .contains("\"operation\": \"update\"")
    );
    command(&temp)
        .args(["integration", "update", "--keybinding", "ctrl-x"])
        .assert()
        .success();
    assert!(!journal.exists());
    assert!(
        fs::read_to_string(temp.path().join("data/hashai/integrations/hashai.bash"))
            .unwrap()
            .contains("__hashai_bash_keybinding='\\C-x'")
    );
}

#[test]
fn ac5_uninstall_never_deletes_an_untracked_exact_artifact() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .success();
    let root = temp.path().join("data/hashai/integrations");
    fs::remove_file(root.join("bash.manifest.json")).unwrap();
    let artifact = root.join("hashai.bash");
    let before = fs::read(&artifact).unwrap();
    command(&temp)
        .args(["integration", "uninstall", "bash"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "bash\tartifact\tmanual-action-required",
        ));
    assert_eq!(fs::read(artifact).unwrap(), before);
}

#[test]
fn ac1_recorded_manifest_path_is_never_used_as_delete_authority() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .success();
    let root = temp.path().join("data/hashai/integrations");
    let managed = root.join("hashai.bash");
    let outside = temp.path().join("outside.bash");
    fs::write(&outside, fs::read(&managed).unwrap()).unwrap();
    let manifest_path = root.join("bash.manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["artifact"]["resolved_path"] =
        serde_json::Value::String(outside.display().to_string());
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    command(&temp)
        .args(["integration", "uninstall", "bash"])
        .assert()
        .code(1);
    assert!(outside.is_file());
    assert!(managed.is_file());
}

#[test]
fn ac1_manifest_and_journal_are_private_regular_files() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .success();
    let root = temp.path().join("data/hashai/integrations");
    let manifest = fs::symlink_metadata(root.join("bash.manifest.json")).unwrap();
    assert!(manifest.file_type().is_file());
    assert_eq!(manifest.permissions().mode() & 0o077, 0);

    command(&temp)
        .env("HASHAI_TEST_INTEGRATION_FAULT_PHASE", "artifact-published")
        .args(["integration", "update", "--keybinding", "ctrl-x"])
        .assert()
        .code(1);
    let journal = fs::symlink_metadata(root.join("bash.journal.json")).unwrap();
    assert!(journal.file_type().is_file());
    assert_eq!(journal.mode() & 0o077, 0);
}

#[test]
fn ac3_bash_snippet_sources_quote_corpus_without_expansion() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data \\ ' $(false) `false` ; 日本語 😀");
    let output = Command::cargo_bin("hashai")
        .unwrap()
        .env("HOME", temp.path().join("home"))
        .env("XDG_DATA_HOME", &data)
        .args(["integration", "install", "bash"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let snippet = stdout
        .split("# hashai snippet begin\n")
        .nth(1)
        .unwrap()
        .split("# hashai snippet end")
        .next()
        .unwrap();
    let status = std::process::Command::new("bash")
        .args(["--noprofile", "--norc", "-c", snippet])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(data.join("hashai/integrations/hashai.bash").is_file());
}

#[test]
fn ac4_shared_lock_blocks_update_without_changing_artifact() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .success();
    let root = temp.path().join("data/hashai/integrations");
    let artifact = root.join("hashai.bash");
    let before = fs::read(&artifact).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join(".hashai-integration.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    command(&temp)
        .args(["integration", "update", "--keybinding", "ctrl-x"])
        .assert()
        .code(1);
    assert_eq!(fs::read(artifact).unwrap(), before);
    FileExt::unlock(&lock).unwrap();
}

#[test]
fn ac5_uninstall_ignores_invalid_user_config_and_starts_no_codex() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .success();
    let config = temp.path().join("config/hashai");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("config.toml"), "not = [valid").unwrap();
    let marker = temp.path().join("codex-started");
    command(&temp)
        .env("HASHAI_CODEX_BIN", format!("touch {}", marker.display()))
        .args(["integration", "uninstall", "bash"])
        .assert()
        .success();
    assert!(!marker.exists());
    assert!(
        !temp
            .path()
            .join("data/hashai/integrations/hashai.bash")
            .exists()
    );
}

#[test]
fn ac5_uninstall_recovery_converges_at_every_removed_phase() {
    for phase in [
        "loader-removed-or-not-applicable",
        "artifact-removed",
        "manifest-removed",
    ] {
        for fault in [format!("before-{phase}"), phase.to_owned()] {
            let temp = tempfile::tempdir().unwrap();
            command(&temp)
                .args(["integration", "install", "fish"])
                .assert()
                .success();
            command(&temp)
                .env("HASHAI_TEST_INTEGRATION_FAULT_PHASE", &fault)
                .args(["integration", "uninstall", "fish"])
                .assert()
                .code(1);
            command(&temp)
                .args(["integration", "uninstall", "fish"])
                .assert()
                .success();
            let root = temp.path().join("data/hashai/integrations");
            assert!(!root.join("fish.journal.json").exists(), "{fault}");
            assert!(!root.join("hashai.fish").exists(), "{fault}");
            assert!(!root.join("fish.manifest.json").exists(), "{fault}");
        }
    }
}

#[test]
fn ac1_untracked_exact_is_adopted_but_modified_and_foreign_are_no_write_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .success();
    let root = temp.path().join("data/hashai/integrations");
    let artifact = root.join("hashai.bash");
    let manifest = root.join("bash.manifest.json");
    fs::remove_file(&manifest).unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .success()
        .stdout(predicates::str::contains("bash\tartifact\tadopted"));
    assert!(manifest.is_file());

    fs::write(&artifact, "modified\n").unwrap();
    let manifest_before = fs::read(&manifest).unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .code(1);
    assert_eq!(fs::read_to_string(&artifact).unwrap(), "modified\n");
    assert_eq!(fs::read(&manifest).unwrap(), manifest_before);
    assert!(!root.join("bash.journal.json").exists());

    fs::write(&manifest, "foreign manifest\n").unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .code(1);
    assert_eq!(fs::read_to_string(&manifest).unwrap(), "foreign manifest\n");
}

#[test]
fn ac1_tracked_prior_updates_and_unsafe_artifact_is_never_replaced() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "zsh"])
        .assert()
        .success();
    command(&temp)
        .args(["integration", "install", "zsh", "--keybinding", "ctrl-x"])
        .assert()
        .success();
    let root = temp.path().join("data/hashai/integrations");
    let artifact = root.join("hashai.zsh");
    assert!(
        fs::read_to_string(&artifact)
            .unwrap()
            .contains("keybinding='^X'")
    );

    command(&temp)
        .args(["integration", "uninstall", "zsh"])
        .assert()
        .success();
    let outside = temp.path().join("outside");
    fs::write(&outside, "outside\n").unwrap();
    symlink(&outside, &artifact).unwrap();
    command(&temp)
        .args(["integration", "install", "zsh"])
        .assert()
        .code(1);
    assert_eq!(fs::read_to_string(outside).unwrap(), "outside\n");
    assert!(!root.join("zsh.manifest.json").exists());
}

#[test]
fn ac2_fish_manual_auto_matrix_and_modified_loader_preflight_are_exact() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .args(["integration", "install", "fish", "--manual"])
        .assert()
        .success();
    let root = temp.path().join("data/hashai/integrations");
    let artifact = root.join("hashai.fish");
    let loader = temp.path().join("config/fish/conf.d/hashai.fish");
    assert!(!loader.exists());
    command(&temp)
        .args(["integration", "install", "fish", "--manual"])
        .assert()
        .success();
    command(&temp)
        .args(["integration", "install", "fish"])
        .assert()
        .success();
    assert!(loader.is_file());

    fs::write(&loader, "modified loader\n").unwrap();
    let artifact_before = fs::read(&artifact).unwrap();
    let manifest_before = fs::read(root.join("fish.manifest.json")).unwrap();
    command(&temp)
        .args([
            "integration",
            "install",
            "fish",
            "--manual",
            "--keybinding",
            "ctrl-x",
        ])
        .assert()
        .code(1);
    assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
    assert_eq!(fs::read_to_string(&loader).unwrap(), "modified loader\n");
    assert_eq!(
        fs::read(root.join("fish.manifest.json")).unwrap(),
        manifest_before
    );
    assert!(!root.join("fish.journal.json").exists());
}

#[test]
fn ac2_fish_auto_to_manual_crash_boundaries_never_leave_a_broken_source() {
    for phase in ["artifact-published", "loader-published-or-not-applicable"] {
        let temp = tempfile::tempdir().unwrap();
        command(&temp)
            .args(["integration", "install", "fish"])
            .assert()
            .success();
        let root = temp.path().join("data/hashai/integrations");
        let artifact = root.join("hashai.fish");
        let loader = temp.path().join("config/fish/conf.d/hashai.fish");
        command(&temp)
            .env("HASHAI_TEST_INTEGRATION_FAULT_PHASE", phase)
            .args([
                "integration",
                "install",
                "fish",
                "--manual",
                "--keybinding",
                "ctrl-x",
            ])
            .assert()
            .code(1);
        assert!(artifact.is_file(), "{phase}");
        if loader.exists() {
            let source = fs::read_to_string(&loader).unwrap();
            assert!(source.contains(artifact.to_str().unwrap()), "{phase}");
        }
        command(&temp)
            .args([
                "integration",
                "install",
                "fish",
                "--manual",
                "--keybinding",
                "ctrl-x",
            ])
            .assert()
            .success();
        assert!(!loader.exists(), "{phase}");
        assert!(
            fs::read_to_string(&artifact)
                .unwrap()
                .contains("keybinding \\cx")
        );
        assert!(!root.join("fish.journal.json").exists(), "{phase}");
    }
}

#[test]
fn ac4_non_utf8_absolute_xdg_config_is_rejected_before_any_write() {
    use std::{
        ffi::OsString,
        os::unix::ffi::{OsStrExt, OsStringExt},
    };
    let temp = tempfile::tempdir().unwrap();
    let mut bytes = temp.path().as_os_str().as_bytes().to_vec();
    bytes.extend_from_slice(b"/config-\xff");
    let invalid = OsString::from_vec(bytes);
    let mut invocation = Command::cargo_bin("hashai").unwrap();
    invocation
        .env("HOME", temp.path().join("home"))
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .env("XDG_CONFIG_HOME", invalid)
        .args(["integration", "install", "fish"])
        .assert()
        .code(2);
    assert!(
        !temp
            .path()
            .join("data/hashai/integrations/hashai.fish")
            .exists()
    );
}

#[test]
fn ac1_manifest_and_journal_schema_roundtrip_fixed_identities_and_hashes() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .env(
            "HASHAI_TEST_INTEGRATION_FAULT_PHASE",
            "loader-published-or-not-applicable",
        )
        .args(["integration", "install", "fish"])
        .assert()
        .code(1);
    let root = temp.path().join("data/hashai/integrations");
    let journal: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("fish.journal.json")).unwrap()).unwrap();
    assert_eq!(journal["format_version"], 1);
    assert_eq!(journal["operation"], "install");
    assert_eq!(journal["shell"], "fish");
    assert_eq!(journal["desired_mode"], "auto");
    assert!(journal["sequence"].as_u64().unwrap() >= 2);
    assert_eq!(
        journal["completed_phase"],
        "loader-published-or-not-applicable"
    );
    assert_eq!(
        journal["resolved_fixed_targets"].as_array().unwrap().len(),
        3
    );

    command(&temp)
        .args(["integration", "install", "fish"])
        .assert()
        .success();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("fish.manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["format_version"], 1);
    assert_eq!(manifest["shell"], "fish");
    assert_eq!(manifest["desired_mode"], "auto");
    for component in ["artifact", "loader"] {
        assert!(
            manifest[component]["resolved_path"]
                .as_str()
                .unwrap()
                .starts_with('/')
        );
        assert_eq!(manifest[component]["sha256"].as_str().unwrap().len(), 64);
    }
}

#[test]
fn ac1_install_recovery_converges_before_and_after_every_marker_update() {
    for fault in ["before-journal-created", "journal-created"] {
        let temp = tempfile::tempdir().unwrap();
        command(&temp)
            .env("HASHAI_TEST_INTEGRATION_FAULT_PHASE", fault)
            .args(["integration", "install", "fish"])
            .assert()
            .code(1);
        command(&temp)
            .args(["integration", "install", "fish"])
            .assert()
            .success();
        let root = temp.path().join("data/hashai/integrations");
        assert!(!root.join("fish.journal.json").exists(), "{fault}");
        assert!(root.join("fish.manifest.json").is_file(), "{fault}");
    }
    for phase in [
        "artifact-published",
        "loader-published-or-not-applicable",
        "manifest-published",
    ] {
        for fault in [format!("before-{phase}"), phase.to_owned()] {
            let temp = tempfile::tempdir().unwrap();
            command(&temp)
                .env("HASHAI_TEST_INTEGRATION_FAULT_PHASE", &fault)
                .args(["integration", "install", "fish"])
                .assert()
                .code(1);
            command(&temp)
                .args(["integration", "install", "fish"])
                .assert()
                .success();
            let root = temp.path().join("data/hashai/integrations");
            assert!(!root.join("fish.journal.json").exists(), "{fault}");
            assert!(root.join("fish.manifest.json").is_file(), "{fault}");
            assert!(
                temp.path().join("config/fish/conf.d/hashai.fish").is_file(),
                "{fault}"
            );
        }
    }
}

#[test]
fn ac1_foreign_journal_is_conflict_and_causes_no_target_write() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data/hashai/integrations");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("bash.journal.json"), "foreign journal\n").unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .code(1);
    assert!(!root.join("hashai.bash").exists());
    assert!(!root.join("bash.manifest.json").exists());
    assert_eq!(
        fs::read_to_string(root.join("bash.journal.json")).unwrap(),
        "foreign journal\n"
    );
}

#[test]
fn ac1_manifest_last_unknown_mismatch_stops_recovery() {
    let temp = tempfile::tempdir().unwrap();
    command(&temp)
        .env("HASHAI_TEST_INTEGRATION_FAULT_PHASE", "manifest-published")
        .args(["integration", "install", "bash"])
        .assert()
        .code(1);
    let root = temp.path().join("data/hashai/integrations");
    let manifest = root.join("bash.manifest.json");
    fs::write(&manifest, "unknown manifest bytes\n").unwrap();
    let artifact_before = fs::read(root.join("hashai.bash")).unwrap();
    command(&temp)
        .args(["integration", "install", "bash"])
        .assert()
        .code(1);
    assert_eq!(
        fs::read_to_string(manifest).unwrap(),
        "unknown manifest bytes\n"
    );
    assert_eq!(fs::read(root.join("hashai.bash")).unwrap(), artifact_before);
    assert!(root.join("bash.journal.json").is_file());
}
