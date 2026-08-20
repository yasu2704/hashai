use std::fs;

use crate::{
    config::{Config, Keybinding, Shell},
    integration::{ARTIFACT_VERSION, IntegrationManager, OwnershipState, WriteOutcome},
};

fn manager(temp: &tempfile::TempDir) -> IntegrationManager {
    IntegrationManager::new(temp.path().join("managed-integrations"))
}

#[test]
fn install_renders_a_versioned_static_artifact_for_every_shell() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        let outcome = manager.install(shell.clone(), true).unwrap();
        assert_eq!(outcome.artifact, WriteOutcome::Written);
        let contents = fs::read_to_string(manager.artifact_path(&shell)).unwrap();
        assert!(contents.contains(&format!("# hashai-integration-version: {ARTIFACT_VERSION}")));
        assert!(contents.contains("does not invoke hashai or codex during shell startup"));
        assert!(contents.contains(&format!("hashai generate --shell {} --", shell.as_str())));
        assert!(!contents.contains("eval"));
    }
}

#[test]
fn install_bakes_validated_editor_settings_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let config = Config {
        trigger: "@@ 日本語 😀 '$()`;".to_owned(),
        keybinding: Keybinding::CtrlX,
        ..Config::default()
    };
    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        assert_eq!(
            manager
                .install_with_config(shell.clone(), &config, true)
                .unwrap()
                .artifact,
            WriteOutcome::Written
        );
        assert_eq!(
            manager
                .install_with_config(shell.clone(), &config, true)
                .unwrap()
                .artifact,
            WriteOutcome::Unchanged
        );
        let contents = fs::read_to_string(manager.artifact_path(&shell)).unwrap();
        assert!(contents.contains("日本語 😀"));
        match shell {
            Shell::Bash => assert!(contents.contains("keybinding='\\C-x'")),
            Shell::Zsh => assert!(contents.contains("keybinding='^X'")),
            Shell::Fish => assert!(contents.contains("keybinding \\cx")),
            Shell::Auto => unreachable!(),
        }
    }
}

#[test]
fn invalid_config_cannot_create_managed_state() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let invalid = Config {
        trigger: "bad\ntrigger".to_owned(),
        ..Config::default()
    };
    assert!(
        manager
            .install_with_config(Shell::Bash, &invalid, true)
            .is_err()
    );
    assert!(!manager.directory().exists());
}

#[test]
fn list_uses_manifest_ownership_not_only_the_version_marker() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    manager.install(Shell::Bash, true).unwrap();
    let artifact = manager.artifact_path(&Shell::Bash);
    fs::write(
        &artifact,
        format!("# hashai-integration-version: {ARTIFACT_VERSION}\nmodified\n"),
    )
    .unwrap();
    let installed = manager.list().unwrap();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].state, OwnershipState::Modified);
    assert!(!installed[0].is_current);
}
