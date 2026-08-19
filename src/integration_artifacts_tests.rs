use std::{
    fs,
    sync::{Arc, Barrier},
};

use crate::{
    config::Shell,
    integration::{ARTIFACT_VERSION, IntegrationManager, UpdateSummary, WriteOutcome},
};
use fs2::FileExt;

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
        match shell {
            Shell::Bash => {
                assert!(contents.contains("__hashai_bash_replace_line"));
                assert!(contents.contains("HASHAI_TRIGGER"));
                assert!(contents.contains("keybinding='\\C-g'"));
                assert!(contents.contains("READLINE_LINE"));
                assert!(contents.contains("READLINE_POINT"));
                assert!(contents.contains("hashai generate --shell bash --"));
                assert!(!contents.contains("eval"));
            }
            Shell::Zsh => {
                assert!(contents.contains("__hashai_zsh_replace_buffer"));
                assert!(contents.contains("HASHAI_TRIGGER"));
                assert!(contents.contains("keybinding='^G'"));
                assert!(contents.contains("bindkey -M emacs \"$keybinding\""));
                assert!(contents.contains("bindkey -M viins \"$keybinding\""));
                assert!(contents.contains("BUFFER"));
                assert!(contents.contains("CURSOR=${#BUFFER}"));
                assert!(contents.contains("hashai generate --shell zsh --"));
                assert!(!contents.contains("eval"));
            }
            Shell::Fish | Shell::Auto => {
                assert!(is_comments_only(&contents), "{}", artifact.display());
            }
        }
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
        UpdateSummary {
            outcomes: vec![(Shell::Bash, WriteOutcome::Written)],
            failures: vec![],
        }
    );
    assert_eq!(fs::read(manager.backup_path(&Shell::Bash)).unwrap(), old);
    assert!(
        fs::read_to_string(&artifact)
            .unwrap()
            .contains(&format!("# hashai-integration-version: {ARTIFACT_VERSION}"))
    );
    assert_eq!(
        manager.update().unwrap(),
        UpdateSummary {
            outcomes: vec![(Shell::Bash, WriteOutcome::Unchanged)],
            failures: vec![],
        }
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

        let summary = manager.update().unwrap();
        assert!(summary.outcomes.is_empty());
        assert_eq!(summary.failures.len(), 1);
        assert_eq!(fs::read(artifact).unwrap(), old);
    }
}

#[test]
fn ac4_ac8_update_continues_safe_shells_and_rerun_converges_after_a_failure() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let bash = manager.artifact_path(&Shell::Bash);
        let zsh = manager.artifact_path(&Shell::Zsh);
        let fish = manager.artifact_path(&Shell::Fish);
        manager.generate(Shell::Bash).unwrap();
        manager.generate(Shell::Zsh).unwrap();
        manager.generate(Shell::Fish).unwrap();
        let old_bash = b"# hashai-integration-version: obsolete\nbash old\n";
        let old_zsh = b"# hashai-integration-version: obsolete\nzsh old\n";
        let old_fish = b"# hashai-integration-version: obsolete\nfish old\n";
        fs::write(&bash, old_bash).unwrap();
        fs::write(&zsh, old_zsh).unwrap();
        fs::write(&fish, old_fish).unwrap();
        symlink(
            temp.path().join("outside"),
            manager.backup_path(&Shell::Zsh),
        )
        .unwrap();

        let summary = manager.update().unwrap();
        assert_eq!(summary.failures.len(), 1);
        assert_eq!(summary.failures[0].shell, Shell::Zsh);
        assert_eq!(
            summary.outcomes,
            vec![
                (Shell::Bash, WriteOutcome::Written),
                (Shell::Fish, WriteOutcome::Written)
            ]
        );
        assert_ne!(fs::read(bash).unwrap(), old_bash);
        assert_eq!(fs::read(zsh).unwrap(), old_zsh);
        assert_ne!(fs::read(fish).unwrap(), old_fish);

        fs::remove_file(manager.backup_path(&Shell::Zsh)).unwrap();
        let retry = manager.update().unwrap();
        assert!(retry.failures.is_empty());
        assert_eq!(
            retry.outcomes,
            vec![
                (Shell::Bash, WriteOutcome::Unchanged),
                (Shell::Zsh, WriteOutcome::Written),
                (Shell::Fish, WriteOutcome::Unchanged),
            ]
        );
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
    assert_eq!(manager.update().unwrap(), UpdateSummary::default());
    assert!(!manager.directory().exists());
}

#[test]
fn ac7_list_and_update_reject_non_directory_and_dangling_directory_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let non_directory_manager = manager(&temp);
    fs::write(non_directory_manager.directory(), "not a directory").unwrap();
    assert!(non_directory_manager.list().is_err());
    assert!(non_directory_manager.update().is_err());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        symlink(temp.path().join("missing-target"), manager.directory()).unwrap();
        assert!(manager.list().is_err());
        assert!(manager.update().is_err());
    }
}

#[test]
fn ac8_cooperating_concurrent_updates_preserve_one_complete_backup_and_converge() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    manager.generate(Shell::Bash).unwrap();
    let artifact = manager.artifact_path(&Shell::Bash);
    let old = b"# hashai-integration-version: obsolete\nconcurrent old\n";
    fs::write(&artifact, old).unwrap();

    let barrier = Arc::new(Barrier::new(4));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let manager = manager.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            manager.update()
        }));
    }
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    let summaries: Vec<_> = results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .collect();

    let written = summaries
        .iter()
        .flat_map(|summary| &summary.outcomes)
        .filter(|(_, outcome)| *outcome == WriteOutcome::Written)
        .count();
    let unchanged = summaries
        .iter()
        .flat_map(|summary| &summary.outcomes)
        .filter(|(_, outcome)| *outcome == WriteOutcome::Unchanged)
        .count();
    assert_eq!(written, 1);
    assert!(written + unchanged <= 4);
    assert!(summaries.iter().all(|summary| summary.failures.is_empty()));
    assert!(results.iter().all(|result| {
        result.as_ref().is_ok_and(|_| true)
            || result
                .as_ref()
                .is_err_and(|error| error.to_string().contains("another integration operation"))
    }));
    assert_eq!(fs::read(manager.backup_path(&Shell::Bash)).unwrap(), old);
    assert!(
        fs::read_to_string(artifact)
            .unwrap()
            .contains(&format!("# hashai-integration-version: {ARTIFACT_VERSION}"))
    );
}

#[test]
fn ac8_contended_write_lock_fails_immediately_without_changing_live_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    manager.generate(Shell::Bash).unwrap();
    let artifact = manager.artifact_path(&Shell::Bash);
    let old = b"# hashai-integration-version: obsolete\nlocked old\n";
    fs::write(&artifact, old).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(manager.directory().join(".hashai-integration.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();

    let error = manager.update().unwrap_err();

    assert!(error.to_string().contains("another integration operation"));
    assert_eq!(fs::read(artifact).unwrap(), old);
    FileExt::unlock(&lock).unwrap();
}

#[test]
fn ac7_write_lock_symlink_is_rejected_without_changing_the_artifact() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        manager.generate(Shell::Bash).unwrap();
        let artifact = manager.artifact_path(&Shell::Bash);
        let old = b"# hashai-integration-version: obsolete\nlock safety\n";
        fs::write(&artifact, old).unwrap();
        let lock = manager.directory().join(".hashai-integration.lock");
        fs::remove_file(&lock).unwrap();
        symlink(temp.path().join("outside"), lock).unwrap();

        assert!(manager.update().is_err());
        assert_eq!(fs::read(artifact).unwrap(), old);
    }
}

fn is_comments_only(contents: &str) -> bool {
    contents
        .lines()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
}
