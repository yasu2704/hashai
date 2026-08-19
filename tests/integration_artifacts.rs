use std::fs;

use hashai::{
    config::Shell,
    integration::{ARTIFACT_VERSION, IntegrationManager, WriteOutcome},
};

fn manager(temp: &tempfile::TempDir) -> IntegrationManager {
    IntegrationManager::new(temp.path().join("managed-integrations"))
}

#[test]
fn ac1_generate_writes_a_versioned_static_artifact_for_every_shell() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);

    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        assert_eq!(
            manager.generate(shell.clone()).unwrap(),
            WriteOutcome::Written
        );
        let artifact = manager.artifact_path(&shell);
        let contents = fs::read_to_string(&artifact).unwrap();
        assert!(contents.contains(&format!("# hashai-integration-version: {ARTIFACT_VERSION}")));
        assert!(contents.contains("does not invoke hashai or codex during shell startup"));
        assert!(is_comments_only(&contents), "{}", artifact.display());
    }
}

#[test]
fn ac2_list_is_read_only_and_reports_current_and_mismatched_versions() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    manager.generate(Shell::Bash).unwrap();
    let bash = manager.artifact_path(&Shell::Bash);
    fs::write(&bash, "# hashai-integration-version: obsolete\n").unwrap();
    let before = fs::read(&bash).unwrap();

    let installed = manager.list().unwrap();

    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].shell, Shell::Bash);
    assert_eq!(installed[0].version.as_deref(), Some("obsolete"));
    assert!(!installed[0].is_current);
    assert_eq!(fs::read(bash).unwrap(), before);
}

#[test]
fn ac3_ac6_update_is_atomic_backs_up_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    manager.generate(Shell::Bash).unwrap();
    let artifact = manager.artifact_path(&Shell::Bash);
    let old = b"# hashai-integration-version: obsolete\n# old bytes: \xff\n";
    fs::write(&artifact, old).unwrap();

    assert_eq!(
        manager.update().unwrap(),
        vec![(Shell::Bash, WriteOutcome::Written)]
    );
    assert_eq!(fs::read(manager.backup_path(&Shell::Bash)).unwrap(), old);
    assert!(
        fs::read_to_string(&artifact)
            .unwrap()
            .contains(&format!("# hashai-integration-version: {ARTIFACT_VERSION}"))
    );
    assert_eq!(
        manager.update().unwrap(),
        vec![(Shell::Bash, WriteOutcome::Unchanged)]
    );
    assert_eq!(
        manager.generate(Shell::Bash).unwrap(),
        WriteOutcome::Unchanged
    );
}

#[test]
fn ac4_failed_update_preserves_existing_artifact_bytes_and_rejects_symlink_backup() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        manager.generate(Shell::Bash).unwrap();
        let artifact = manager.artifact_path(&Shell::Bash);
        let old = b"# hashai-integration-version: obsolete\nunchanged bytes\n";
        fs::write(&artifact, old).unwrap();
        symlink(
            temp.path().join("outside"),
            manager.backup_path(&Shell::Bash),
        )
        .unwrap();

        assert!(manager.update().is_err());
        assert_eq!(fs::read(artifact).unwrap(), old);
    }
}

#[test]
fn ac7_management_paths_are_shell_specific_and_stay_under_the_managed_directory() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        let artifact = manager.artifact_path(&shell);
        assert!(artifact.starts_with(manager.directory()));
        assert_eq!(
            artifact.file_name().and_then(|name| name.to_str()),
            Some(format!("hashai.{}", shell.as_str()).as_str())
        );
    }
}

#[test]
fn ac8_list_and_update_handle_no_installed_artifacts_without_writing() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    assert!(manager.list().unwrap().is_empty());
    assert!(manager.update().unwrap().is_empty());
    assert!(!manager.directory().exists());
}

fn is_comments_only(contents: &str) -> bool {
    contents
        .lines()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
}
