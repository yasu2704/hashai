//! Static, versioned shell integration artifacts.
//!
//! Artifacts live only in hashai's user data directory.  This module never
//! evaluates an artifact, starts Core, or invokes Codex; shell-editor behavior
//! is deliberately left to the shell-specific Phase 2 issues.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use directories::ProjectDirs;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::{
    HashaiError,
    config::{Config, Keybinding, Shell, validate},
};

pub const ARTIFACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const VERSION_MARKER: &str = "# hashai-integration-version: ";
const BASH_INTEGRATION: &str = include_str!("../shell/hashai.bash");
const ZSH_INTEGRATION: &str = include_str!("../shell/hashai.zsh");
const FISH_INTEGRATION: &str = include_str!("../shell/hashai.fish");

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WriteOutcome {
    Written,
    Unchanged,
    Adopted,
    Removed,
    Absent,
    ManualActionRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnershipState {
    Absent,
    UntrackedExactExpected,
    TrackedExact,
    TrackedPrior,
    Modified,
    Foreign,
    Unsafe,
    Unreadable,
    InterruptedRecoverable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrationInspection {
    pub artifact: OwnershipState,
    pub loader: Option<OwnershipState>,
    pub desired_mode: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct ManagedFile {
    resolved_path: String,
    sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct OwnershipManifest {
    format_version: u32,
    shell: String,
    artifact: ManagedFile,
    loader: Option<ManagedFile>,
    desired_mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct JournalTarget {
    resolved_path: String,
    pre_sha256: Option<String>,
    desired_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct TransactionJournal {
    format_version: u32,
    operation: String,
    shell: String,
    generation_id: String,
    sequence: u32,
    resolved_fixed_targets: Vec<JournalTarget>,
    desired_mode: String,
    completed_phase: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallOutcome {
    pub artifact: WriteOutcome,
    pub loader: Option<WriteOutcome>,
    pub manifest: WriteOutcome,
    pub artifact_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UninstallOutcome {
    pub artifact: WriteOutcome,
    pub loader: Option<WriteOutcome>,
    pub manifest: WriteOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledIntegration {
    pub shell: Shell,
    pub path: PathBuf,
    pub version: Option<String>,
    pub is_current: bool,
    pub state: OwnershipState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateFailure {
    pub shell: Shell,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpdateSummary {
    pub outcomes: Vec<(Shell, InstallOutcome)>,
    pub failures: Vec<UpdateFailure>,
}

#[derive(Clone, Debug)]
pub struct IntegrationManager {
    directory: PathBuf,
}

impl IntegrationManager {
    pub(crate) fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn from_system() -> Result<Self, HashaiError> {
        let directories = ProjectDirs::from("com", "yasu2704", "hashai").ok_or_else(|| {
            HashaiError::ArgumentOrConfig(
                "could not determine user data directory for integrations".to_owned(),
            )
        })?;
        let xdg_data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
        let directory = resolve_integration_directory(
            xdg_data_home.as_deref(),
            directories.data_local_dir().join("integrations"),
        );
        Ok(Self::new(lexical_absolute_path(&directory)?))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn artifact_path(&self, shell: &Shell) -> PathBuf {
        self.directory.join(format!("hashai.{}", shell.as_str()))
    }

    pub fn install(&self, shell: Shell, manual: bool) -> Result<InstallOutcome, HashaiError> {
        self.install_with_config(shell, &Config::default(), manual)
    }

    pub fn install_with_config(
        &self,
        shell: Shell,
        config: &Config,
        manual: bool,
    ) -> Result<InstallOutcome, HashaiError> {
        validate_integration_shell(&shell)?;
        validate(config)?;
        self.with_write_lock(|| self.install_locked("install", &shell, config, manual))
    }

    pub fn uninstall(&self, shell: Shell) -> Result<UninstallOutcome, HashaiError> {
        validate_integration_shell(&shell)?;
        if self.managed_directory_is_absent()? {
            return Ok(UninstallOutcome {
                artifact: WriteOutcome::Absent,
                loader: None,
                manifest: WriteOutcome::Absent,
            });
        }
        self.with_write_lock(|| self.uninstall_locked(&shell))
    }

    pub fn manifest_path(&self, shell: &Shell) -> PathBuf {
        self.directory
            .join(format!("{}.manifest.json", shell.as_str()))
    }

    pub fn inspect(
        &self,
        shell: &Shell,
        config: &Config,
    ) -> Result<IntegrationInspection, HashaiError> {
        validate_integration_shell(shell)?;
        validate(config)?;
        let artifact_path = self.artifact_path(shell);
        let expected = artifact_contents(shell, config);
        let manifest_path = self.manifest_path(shell);
        let journal_path = self.journal_path(shell);
        let journal = inspect_json::<TransactionJournal>(&journal_path);
        if matches!(journal, FileInspection::Unsafe) {
            return Ok(IntegrationInspection {
                artifact: OwnershipState::Unsafe,
                loader: None,
                desired_mode: None,
            });
        }
        if matches!(journal, FileInspection::Unreadable) {
            return Ok(IntegrationInspection {
                artifact: OwnershipState::Unreadable,
                loader: None,
                desired_mode: None,
            });
        }
        let manifest = inspect_json::<OwnershipManifest>(&manifest_path);
        let artifact_file = inspect_bytes(&artifact_path);
        let mut artifact = classify_artifact(
            shell,
            &artifact_path,
            expected.as_bytes(),
            &manifest,
            &artifact_file,
        );
        if let FileInspection::Present(journal) = &journal {
            artifact = if journal_is_recoverable(journal) {
                OwnershipState::InterruptedRecoverable
            } else {
                OwnershipState::Foreign
            };
        }
        let (loader, desired_mode) = match &manifest {
            FileInspection::Present(manifest) => {
                let state = match &manifest.loader {
                    Some(loader) if *shell != Shell::Fish => Some(OwnershipState::Foreign),
                    Some(loader) => {
                        let expected_path = resolve_fish_loader_path()?;
                        if loader.resolved_path != path_string(&expected_path)? {
                            Some(OwnershipState::Foreign)
                        } else {
                            let expected = fish_loader_contents(&artifact_path);
                            Some(classify_tracked_file(loader, Some(expected.as_bytes())))
                        }
                    }
                    None if manifest.desired_mode == "manual" => Some(OwnershipState::Absent),
                    None => Some(OwnershipState::Foreign),
                };
                (state, Some(manifest.desired_mode.clone()))
            }
            _ if *shell == Shell::Fish => (Some(OwnershipState::Absent), None),
            _ => (None, None),
        };
        Ok(IntegrationInspection {
            artifact,
            loader,
            desired_mode,
        })
    }

    fn journal_path(&self, shell: &Shell) -> PathBuf {
        self.directory
            .join(format!("{}.journal.json", shell.as_str()))
    }

    fn prepare_journal(
        &self,
        operation: &str,
        shell: &Shell,
        desired: &OwnershipManifest,
        manifest_bytes: &[u8],
        prior: Option<&OwnershipManifest>,
    ) -> Result<TransactionJournal, HashaiError> {
        let path = self.journal_path(shell);
        let desired_loader = desired.loader.as_ref();
        let prior_loader = prior.and_then(|value| value.loader.as_ref());
        let loader = desired_loader.or(prior_loader);
        validate_existing_ancestors(Path::new(&desired.artifact.resolved_path))?;
        validate_existing_ancestors(&self.manifest_path(shell))?;
        if let Some(file) = loader {
            validate_existing_ancestors(Path::new(&file.resolved_path))?;
        }
        if let Some(existing) =
            read_json_optional::<TransactionJournal>(&path, "integration journal")?
        {
            if existing.operation != operation || existing.shell != shell.as_str() {
                return Err(HashaiError::Integration(
                    "foreign or stale integration journal conflicts with this operation".to_owned(),
                ));
            }
            let mut expected_targets = vec![(
                desired.artifact.resolved_path.as_str(),
                Some(desired.artifact.sha256.as_str()),
            )];
            if let Some(file) = loader {
                expected_targets.push((
                    file.resolved_path.as_str(),
                    desired_loader.map(|value| value.sha256.as_str()),
                ));
            }
            let manifest_path = path_string(&self.manifest_path(shell))?;
            let manifest_hash = sha256(manifest_bytes);
            expected_targets.push((manifest_path.as_str(), Some(manifest_hash.as_str())));
            if existing.resolved_fixed_targets.len() != expected_targets.len()
                || existing
                    .resolved_fixed_targets
                    .iter()
                    .zip(expected_targets)
                    .any(|(actual, expected)| {
                        actual.resolved_path != expected.0
                            || actual.desired_sha256.as_deref() != expected.1
                    })
            {
                return Err(HashaiError::Integration(
                    "journal target identity conflict; no files changed".to_owned(),
                ));
            }
            for target in &existing.resolved_fixed_targets {
                let current = current_hash(Path::new(&target.resolved_path))?;
                if current != target.pre_sha256 && current != target.desired_sha256 {
                    return Err(HashaiError::Integration(
                        "interrupted transaction target no longer matches pre-state or desired state"
                            .to_owned(),
                    ));
                }
            }
            return Ok(existing);
        }
        let artifact_current = current_hash(Path::new(&desired.artifact.resolved_path))?;
        let artifact_allowed = match prior {
            Some(value) => {
                artifact_current.as_ref() == Some(&value.artifact.sha256)
                    || artifact_current.as_ref() == Some(&desired.artifact.sha256)
            }
            None => {
                artifact_current.is_none()
                    || artifact_current.as_ref() == Some(&desired.artifact.sha256)
            }
        };
        if !artifact_allowed {
            return Err(HashaiError::Integration(
                "integration artifact preflight conflict; no files changed".to_owned(),
            ));
        }
        let mut targets = vec![JournalTarget {
            resolved_path: desired.artifact.resolved_path.clone(),
            pre_sha256: artifact_current,
            desired_sha256: Some(desired.artifact.sha256.clone()),
        }];
        if let Some(file) = loader {
            let current = current_hash(Path::new(&file.resolved_path))?;
            let allowed = match prior_loader {
                Some(value) => {
                    current.as_ref() == Some(&value.sha256)
                        || desired_loader
                            .is_some_and(|desired| current.as_ref() == Some(&desired.sha256))
                }
                None => current.is_none(),
            };
            if !allowed {
                return Err(HashaiError::Integration(
                    "integration loader preflight conflict; no files changed".to_owned(),
                ));
            }
            targets.push(JournalTarget {
                resolved_path: file.resolved_path.clone(),
                pre_sha256: current,
                desired_sha256: desired_loader.map(|value| value.sha256.clone()),
            });
        }
        targets.push(JournalTarget {
            resolved_path: path_string(&self.manifest_path(shell))?,
            pre_sha256: current_hash(&self.manifest_path(shell))?,
            desired_sha256: Some(sha256(manifest_bytes)),
        });
        let generation = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let journal = TransactionJournal {
            format_version: 1,
            operation: operation.to_owned(),
            shell: shell.as_str().to_owned(),
            generation_id: format!("{:x}-{:x}", std::process::id(), generation),
            sequence: 0,
            resolved_fixed_targets: targets,
            desired_mode: desired.desired_mode.clone(),
            completed_phase: "journal-created".to_owned(),
        };
        #[cfg(debug_assertions)]
        if std::env::var("HASHAI_TEST_INTEGRATION_FAULT_PHASE").as_deref()
            == Ok("before-journal-created")
        {
            return Err(HashaiError::Integration(
                "injected integration transaction interruption".to_owned(),
            ));
        }
        write_json(&path, &journal)?;
        #[cfg(debug_assertions)]
        if std::env::var("HASHAI_TEST_INTEGRATION_FAULT_PHASE").as_deref() == Ok("journal-created")
        {
            return Err(HashaiError::Integration(
                "injected integration transaction interruption".to_owned(),
            ));
        }
        Ok(journal)
    }

    fn advance_journal(
        &self,
        shell: &Shell,
        journal: &mut TransactionJournal,
        phase: &str,
    ) -> Result<(), HashaiError> {
        #[cfg(debug_assertions)]
        if std::env::var("HASHAI_TEST_INTEGRATION_FAULT_PHASE").as_deref()
            == Ok(format!("before-{phase}").as_str())
        {
            return Err(HashaiError::Integration(
                "injected integration transaction interruption".to_owned(),
            ));
        }
        journal.sequence += 1;
        journal.completed_phase = phase.to_owned();
        write_json(&self.journal_path(shell), journal)?;
        #[cfg(debug_assertions)]
        if std::env::var("HASHAI_TEST_INTEGRATION_FAULT_PHASE").as_deref() == Ok(phase) {
            return Err(HashaiError::Integration(
                "injected integration transaction interruption".to_owned(),
            ));
        }
        Ok(())
    }

    fn prepare_uninstall_journal(
        &self,
        shell: &Shell,
        manifest: Option<&OwnershipManifest>,
    ) -> Result<Option<TransactionJournal>, HashaiError> {
        let journal_path = self.journal_path(shell);
        if let Some(existing) =
            read_json_optional::<TransactionJournal>(&journal_path, "integration journal")?
        {
            if existing.operation != "uninstall" || existing.shell != shell.as_str() {
                return Err(HashaiError::Integration(
                    "foreign integration journal conflicts with uninstall".to_owned(),
                ));
            }
            let artifact = path_string(&self.artifact_path(shell))?;
            let manifest = path_string(&self.manifest_path(shell))?;
            let expected = if existing.desired_mode == "auto" {
                vec![
                    path_string(&resolve_fish_loader_path()?)?,
                    artifact,
                    manifest,
                ]
            } else {
                vec![artifact, manifest]
            };
            if existing
                .resolved_fixed_targets
                .iter()
                .map(|target| target.resolved_path.as_str())
                .ne(expected.iter().map(String::as_str))
            {
                return Err(HashaiError::Integration(
                    "journal target identity conflict; no files changed".to_owned(),
                ));
            }
            return Ok(Some(existing));
        }
        let Some(manifest) = manifest else {
            return Ok(None);
        };
        let mut targets = Vec::new();
        if let Some(loader) = &manifest.loader {
            targets.push(JournalTarget {
                resolved_path: loader.resolved_path.clone(),
                pre_sha256: Some(loader.sha256.clone()),
                desired_sha256: None,
            });
        }
        targets.push(JournalTarget {
            resolved_path: manifest.artifact.resolved_path.clone(),
            pre_sha256: Some(manifest.artifact.sha256.clone()),
            desired_sha256: None,
        });
        targets.push(JournalTarget {
            resolved_path: path_string(&self.manifest_path(shell))?,
            pre_sha256: current_hash(&self.manifest_path(shell))?,
            desired_sha256: None,
        });
        for target in &targets {
            let current = current_hash(Path::new(&target.resolved_path))?;
            if current != target.pre_sha256 {
                return Err(HashaiError::Integration(
                    "managed uninstall target is modified; no files changed".to_owned(),
                ));
            }
        }
        let generation = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let journal = TransactionJournal {
            format_version: 1,
            operation: "uninstall".to_owned(),
            shell: shell.as_str().to_owned(),
            generation_id: format!("{:x}-{:x}", std::process::id(), generation),
            sequence: 0,
            resolved_fixed_targets: targets,
            desired_mode: manifest.desired_mode.clone(),
            completed_phase: "journal-created".to_owned(),
        };
        #[cfg(debug_assertions)]
        if std::env::var("HASHAI_TEST_INTEGRATION_FAULT_PHASE").as_deref()
            == Ok("before-journal-created")
        {
            return Err(HashaiError::Integration(
                "injected integration transaction interruption".to_owned(),
            ));
        }
        write_json(&journal_path, &journal)?;
        #[cfg(debug_assertions)]
        if std::env::var("HASHAI_TEST_INTEGRATION_FAULT_PHASE").as_deref() == Ok("journal-created")
        {
            return Err(HashaiError::Integration(
                "injected integration transaction interruption".to_owned(),
            ));
        }
        Ok(Some(journal))
    }

    fn install_locked(
        &self,
        operation: &str,
        shell: &Shell,
        config: &Config,
        manual: bool,
    ) -> Result<InstallOutcome, HashaiError> {
        let artifact_path = self.artifact_path(shell);
        let expected = artifact_contents(shell, config);
        let manifest_path = self.manifest_path(shell);
        let prior_manifest = read_manifest_optional(&manifest_path)?;
        if let Some(manifest) = &prior_manifest {
            self.validate_manifest_identity(shell, manifest)?;
        }
        let loader_path = if *shell == Shell::Fish && !manual {
            Some(resolve_fish_loader_path()?)
        } else {
            None
        };
        let loader_contents = loader_path
            .as_ref()
            .map(|_| fish_loader_contents(&artifact_path));
        let manifest = OwnershipManifest {
            format_version: 1,
            shell: shell.as_str().to_owned(),
            artifact: managed_file(&artifact_path, expected.as_bytes())?,
            loader: loader_path
                .as_ref()
                .zip(loader_contents.as_ref())
                .map(|(path, bytes)| managed_file(path, bytes.as_bytes()))
                .transpose()?,
            desired_mode: if loader_path.is_some() {
                "auto"
            } else {
                "manual"
            }
            .to_owned(),
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| HashaiError::Integration(error.to_string()))?;
        let mut journal = self.prepare_journal(
            operation,
            shell,
            &manifest,
            &manifest_bytes,
            prior_manifest.as_ref(),
        )?;
        let artifact_outcome = match fs::symlink_metadata(&artifact_path) {
            Ok(_) => {
                ensure_regular_file(&artifact_path, "integration artifact")?;
                let bytes = fs::read(&artifact_path)?;
                if bytes == expected.as_bytes() {
                    if prior_manifest.is_some() {
                        WriteOutcome::Unchanged
                    } else {
                        WriteOutcome::Adopted
                    }
                } else if let Some(manifest) = &prior_manifest {
                    if sha256(&bytes) != manifest.artifact.sha256 {
                        return Err(HashaiError::Integration(
                            "integration artifact is modified; no files changed".to_owned(),
                        ));
                    }
                    atomic_write(&self.directory, &artifact_path, expected.as_bytes())?;
                    WriteOutcome::Written
                } else {
                    return Err(HashaiError::Integration(
                        "foreign integration artifact; delete it manually before installing"
                            .to_owned(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                atomic_write(&self.directory, &artifact_path, expected.as_bytes())?;
                WriteOutcome::Written
            }
            Err(error) => return Err(error.into()),
        };
        self.advance_journal(shell, &mut journal, "artifact-published")?;
        let loader_outcome = if let Some(path) = &loader_path {
            ensure_safe_parent(path)?;
            Some(write_exact_file(
                path,
                loader_contents
                    .as_ref()
                    .expect("loader contents")
                    .as_bytes(),
            )?)
        } else if let Some(prior_loader) = prior_manifest
            .as_ref()
            .and_then(|value| value.loader.as_ref())
        {
            Some(remove_exact(prior_loader)?)
        } else {
            None
        };
        self.advance_journal(shell, &mut journal, "loader-published-or-not-applicable")?;
        let manifest_outcome = write_exact_file(&manifest_path, &manifest_bytes)?;
        self.advance_journal(shell, &mut journal, "manifest-published")?;
        fs::remove_file(self.journal_path(shell))?;
        Ok(InstallOutcome {
            artifact: artifact_outcome,
            loader: loader_outcome,
            manifest: manifest_outcome,
            artifact_path,
        })
    }

    fn uninstall_locked(&self, shell: &Shell) -> Result<UninstallOutcome, HashaiError> {
        let manifest_path = self.manifest_path(shell);
        let manifest = read_manifest_optional(&manifest_path)?;
        if let Some(manifest) = &manifest {
            self.validate_manifest_identity(shell, manifest)?;
        }
        let Some(mut journal) = self.prepare_uninstall_journal(shell, manifest.as_ref())? else {
            let artifact = match inspect_bytes(&self.artifact_path(shell)) {
                FileInspection::Absent => WriteOutcome::Absent,
                FileInspection::Present(_) => WriteOutcome::ManualActionRequired,
                FileInspection::Unsafe => {
                    return Err(HashaiError::Integration(
                        "untracked integration artifact is unsafe; not removed".to_owned(),
                    ));
                }
                FileInspection::Unreadable | FileInspection::Foreign => {
                    return Err(HashaiError::Integration(
                        "untracked integration artifact is unreadable; not removed".to_owned(),
                    ));
                }
            };
            return Ok(UninstallOutcome {
                artifact,
                loader: None,
                manifest: WriteOutcome::Absent,
            });
        };
        let has_loader = journal.resolved_fixed_targets.len() == 3;
        let mut index = 0;
        let loader = if has_loader {
            let outcome = remove_journal_target(&journal.resolved_fixed_targets[index])?;
            index += 1;
            self.advance_journal(shell, &mut journal, "loader-removed-or-not-applicable")?;
            Some(outcome)
        } else {
            self.advance_journal(shell, &mut journal, "loader-removed-or-not-applicable")?;
            None
        };
        let artifact = remove_journal_target(&journal.resolved_fixed_targets[index])?;
        index += 1;
        self.advance_journal(shell, &mut journal, "artifact-removed")?;
        let manifest_outcome = remove_journal_target(&journal.resolved_fixed_targets[index])?;
        self.advance_journal(shell, &mut journal, "manifest-removed")?;
        fs::remove_file(self.journal_path(shell))?;
        Ok(UninstallOutcome {
            artifact,
            loader,
            manifest: manifest_outcome,
        })
    }

    fn validate_manifest_identity(
        &self,
        shell: &Shell,
        manifest: &OwnershipManifest,
    ) -> Result<(), HashaiError> {
        if manifest.format_version != 1
            || manifest.shell != shell.as_str()
            || manifest.artifact.resolved_path != path_string(&self.artifact_path(shell))?
            || !matches!(manifest.desired_mode.as_str(), "auto" | "manual")
        {
            return Err(HashaiError::Integration(
                "manifest identity conflict; no files changed".to_owned(),
            ));
        }
        match (&manifest.loader, shell, manifest.desired_mode.as_str()) {
            (Some(loader), Shell::Fish, "auto")
                if loader.resolved_path == path_string(&resolve_fish_loader_path()?)? =>
            {
                Ok(())
            }
            (None, Shell::Fish, "manual") | (None, Shell::Bash | Shell::Zsh, "manual") => Ok(()),
            _ => Err(HashaiError::Integration(
                "manifest loader identity conflict; no files changed".to_owned(),
            )),
        }
    }

    pub fn update(&self) -> Result<UpdateSummary, HashaiError> {
        self.update_with_config(&Config::default())
    }

    pub fn update_with_config(&self, config: &Config) -> Result<UpdateSummary, HashaiError> {
        // This must precede even absence checks/locking so an invalid public
        // Config can never create a directory, lock, journal, or partial update.
        validate(config)?;
        if self.managed_directory_is_absent()? {
            return Ok(UpdateSummary::default());
        }
        self.with_write_lock(|| self.update_locked(config))
    }

    fn update_locked(&self, config: &Config) -> Result<UpdateSummary, HashaiError> {
        let mut summary = UpdateSummary::default();
        for shell in supported_shells() {
            let artifact = self.artifact_path(&shell);
            if matches!(fs::symlink_metadata(&artifact), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
                && matches!(fs::symlink_metadata(self.manifest_path(&shell)), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
                && matches!(fs::symlink_metadata(self.journal_path(&shell)), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
            {
                continue;
            }
            let expected = artifact_contents(&shell, config);
            let manifest = match read_manifest_optional(&self.manifest_path(&shell)) {
                Ok(value) => value,
                Err(error) => {
                    summary.failures.push(UpdateFailure {
                        shell,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            if manifest.is_none() {
                if let Some(journal) = match read_json_optional::<TransactionJournal>(
                    &self.journal_path(&shell),
                    "integration journal",
                ) {
                    Ok(value) => value,
                    Err(error) => {
                        summary.failures.push(UpdateFailure {
                            shell,
                            message: error.to_string(),
                        });
                        continue;
                    }
                } {
                    let manual = journal.desired_mode == "manual";
                    match self.install_locked("update", &shell, config, manual) {
                        Ok(outcome) => summary.outcomes.push((shell, outcome)),
                        Err(error) => summary.failures.push(UpdateFailure {
                            shell,
                            message: error.to_string(),
                        }),
                    }
                    continue;
                }
                match fs::read(&artifact) {
                    Ok(bytes) if bytes == expected.as_bytes() => summary.outcomes.push((shell, InstallOutcome {
                        artifact: WriteOutcome::Unchanged,
                        loader: None,
                        manifest: WriteOutcome::Absent,
                        artifact_path: artifact.clone(),
                    })),
                    Ok(_) => summary.failures.push(UpdateFailure { shell, message: "untracked integration artifact conflicts; delete it manually and reinstall".to_owned() }),
                    Err(_) => summary.failures.push(UpdateFailure { shell, message: "integration artifact is unreadable".to_owned() }),
                }
                continue;
            }
            let manual = manifest
                .as_ref()
                .is_some_and(|value| value.desired_mode == "manual");
            match self.install_locked("update", &shell, config, manual) {
                Ok(outcome) => summary.outcomes.push((shell, outcome)),
                Err(error) => summary.failures.push(UpdateFailure {
                    shell,
                    message: error.to_string(),
                }),
            }
        }
        Ok(summary)
    }

    pub fn list(&self) -> Result<Vec<InstalledIntegration>, HashaiError> {
        self.list_with_config(&Config::default())
    }

    pub fn list_with_config(
        &self,
        config: &Config,
    ) -> Result<Vec<InstalledIntegration>, HashaiError> {
        validate(config)?;
        if self.managed_directory_is_absent()? {
            return Ok(Vec::new());
        }
        self.ensure_managed_directory()?;

        let mut installed = Vec::new();
        for shell in supported_shells() {
            let path = self.artifact_path(&shell);
            let manifest_path = self.manifest_path(&shell);
            let journal_path = self.journal_path(&shell);
            if matches!(fs::symlink_metadata(&path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
                && matches!(fs::symlink_metadata(&manifest_path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
                && matches!(fs::symlink_metadata(&journal_path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
            {
                continue;
            }
            let state = self.inspect(&shell, config)?.artifact;
            let is_current = state == OwnershipState::TrackedExact;
            let version = fs::read_to_string(&path)
                .ok()
                .and_then(|contents| artifact_version(&contents));
            installed.push(InstalledIntegration {
                shell,
                path,
                version,
                is_current,
                state,
            });
        }
        Ok(installed)
    }

    fn with_write_lock<T>(
        &self,
        operation: impl FnOnce() -> Result<T, HashaiError>,
    ) -> Result<T, HashaiError> {
        self.ensure_managed_directory()?;
        let path = self.directory.join(".hashai-integration.lock");
        if fs::symlink_metadata(&path).is_ok() {
            ensure_regular_file(&path, "integration lock")?;
        }
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let file = options.open(&path)?;
        ensure_regular_file(&path, "integration lock")?;
        file.try_lock_exclusive().map_err(|_| {
            HashaiError::Integration(
                "another integration operation is in progress; retry after it finishes".to_owned(),
            )
        })?;
        operation()
    }

    fn ensure_managed_directory(&self) -> Result<(), HashaiError> {
        match fs::symlink_metadata(&self.directory) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(HashaiError::Integration(
                        "managed integration directory must be a real directory".to_owned(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&self.directory)?;
                let metadata = fs::symlink_metadata(&self.directory)?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(HashaiError::Integration(
                        "managed integration directory is unsafe".to_owned(),
                    ));
                }
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn managed_directory_is_absent(&self) -> Result<bool, HashaiError> {
        match fs::symlink_metadata(&self.directory) {
            Ok(_) => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(_) => Err(HashaiError::Integration(
                "managed integration directory is unreadable".to_owned(),
            )),
        }
    }
}

fn resolve_integration_directory(xdg_data_home: Option<&Path>, fallback: PathBuf) -> PathBuf {
    match xdg_data_home.filter(|path| path.is_absolute()) {
        Some(data_home) => data_home.join("hashai/integrations"),
        None => fallback,
    }
}

fn resolve_fish_loader_path() -> Result<PathBuf, HashaiError> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.filter(|path| path.is_absolute())
                .map(|path| path.join(".config"))
        })
        .ok_or_else(|| {
            HashaiError::ArgumentOrConfig("HOME must be an absolute Unicode path".to_owned())
        })?;
    if base.to_str().is_none() {
        return Err(HashaiError::ArgumentOrConfig(
            "configuration path must be Unicode".to_owned(),
        ));
    }
    lexical_absolute_path(&base.join("fish/conf.d/hashai.fish"))
}

fn lexical_absolute_path(path: &Path) -> Result<PathBuf, HashaiError> {
    use std::path::Component;
    if !path.is_absolute() || path.to_str().is_none() {
        return Err(HashaiError::ArgumentOrConfig(
            "managed path must be absolute Unicode".to_owned(),
        ));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() || normalized.as_os_str().is_empty() {
                    normalized.push(std::path::MAIN_SEPARATOR_STR);
                }
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    Ok(normalized)
}

fn fish_loader_contents(artifact: &Path) -> String {
    let value = artifact
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('\'', "\\'");
    format!(
        "# hashai-loader-format: 1\nsource '{value}'\nif status is-interactive\n    function __hashai_fish_loader_activate --on-event fish_prompt\n        __hashai_fish_install_binding\n        functions -e __hashai_fish_loader_activate\n    end\nend\n"
    )
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn path_string(path: &Path) -> Result<String, HashaiError> {
    if !path.is_absolute() {
        return Err(HashaiError::ArgumentOrConfig(
            "managed path must be absolute".to_owned(),
        ));
    }
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| HashaiError::ArgumentOrConfig("managed path must be Unicode".to_owned()))
}

fn managed_file(path: &Path, bytes: &[u8]) -> Result<ManagedFile, HashaiError> {
    Ok(ManagedFile {
        resolved_path: path_string(path)?,
        sha256: sha256(bytes),
    })
}

fn read_manifest_optional(path: &Path) -> Result<Option<OwnershipManifest>, HashaiError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            ensure_regular_file(path, "integration manifest")?;
            let bytes = fs::read(path)?;
            serde_json::from_slice(&bytes).map(Some).map_err(|_| {
                HashaiError::Integration(
                    "foreign integration manifest; no files changed".to_owned(),
                )
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

enum FileInspection<T> {
    Absent,
    Present(T),
    Unsafe,
    Unreadable,
    Foreign,
}

fn inspect_json<T: for<'de> Deserialize<'de>>(path: &Path) -> FileInspection<T> {
    match inspect_bytes(path) {
        FileInspection::Absent => FileInspection::Absent,
        FileInspection::Unsafe => FileInspection::Unsafe,
        FileInspection::Unreadable => FileInspection::Unreadable,
        FileInspection::Foreign => FileInspection::Foreign,
        FileInspection::Present(bytes) => serde_json::from_slice(&bytes)
            .map(FileInspection::Present)
            .unwrap_or(FileInspection::Foreign),
    }
}

fn inspect_bytes(path: &Path) -> FileInspection<Vec<u8>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            FileInspection::Unsafe
        }
        Ok(_) => match fs::read(path) {
            Ok(bytes) => FileInspection::Present(bytes),
            Err(_) => FileInspection::Unreadable,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileInspection::Absent,
        Err(_) => FileInspection::Unreadable,
    }
}

fn classify_artifact(
    shell: &Shell,
    path: &Path,
    expected: &[u8],
    manifest: &FileInspection<OwnershipManifest>,
    artifact: &FileInspection<Vec<u8>>,
) -> OwnershipState {
    match (manifest, artifact) {
        (FileInspection::Unsafe, _) | (_, FileInspection::Unsafe) => OwnershipState::Unsafe,
        (FileInspection::Unreadable, _) | (_, FileInspection::Unreadable) => {
            OwnershipState::Unreadable
        }
        (FileInspection::Foreign, _) => OwnershipState::Foreign,
        (FileInspection::Absent, FileInspection::Absent) => OwnershipState::Absent,
        (FileInspection::Absent, FileInspection::Present(bytes)) if bytes == expected => {
            OwnershipState::UntrackedExactExpected
        }
        (FileInspection::Absent, FileInspection::Present(_)) => OwnershipState::Foreign,
        (FileInspection::Present(manifest), FileInspection::Present(bytes)) => {
            if manifest.format_version != 1
                || manifest.shell != shell.as_str()
                || manifest.artifact.resolved_path != path.to_string_lossy()
            {
                OwnershipState::Foreign
            } else if sha256(bytes) != manifest.artifact.sha256 {
                OwnershipState::Modified
            } else if bytes == expected {
                OwnershipState::TrackedExact
            } else {
                OwnershipState::TrackedPrior
            }
        }
        (FileInspection::Present(_), FileInspection::Absent) => OwnershipState::Modified,
        (_, FileInspection::Foreign) => OwnershipState::Foreign,
    }
}

fn classify_tracked_file(file: &ManagedFile, expected: Option<&[u8]>) -> OwnershipState {
    match inspect_bytes(Path::new(&file.resolved_path)) {
        FileInspection::Absent => OwnershipState::Modified,
        FileInspection::Present(bytes) if sha256(&bytes) == file.sha256 => {
            if expected.is_none_or(|expected| bytes == expected) {
                OwnershipState::TrackedExact
            } else {
                OwnershipState::TrackedPrior
            }
        }
        FileInspection::Present(_) => OwnershipState::Modified,
        FileInspection::Unsafe => OwnershipState::Unsafe,
        FileInspection::Unreadable => OwnershipState::Unreadable,
        FileInspection::Foreign => OwnershipState::Foreign,
    }
}

fn journal_is_recoverable(journal: &TransactionJournal) -> bool {
    journal.format_version == 1
        && matches!(
            journal.operation.as_str(),
            "install" | "update" | "uninstall"
        )
        && journal.resolved_fixed_targets.iter().all(|target| {
            let current = current_hash(Path::new(&target.resolved_path));
            current.is_ok_and(|hash| hash == target.pre_sha256 || hash == target.desired_sha256)
        })
}

fn read_json_optional<T: for<'de> Deserialize<'de>>(
    path: &Path,
    label: &str,
) -> Result<Option<T>, HashaiError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            ensure_regular_file(path, label)?;
            serde_json::from_slice(&fs::read(path)?)
                .map(Some)
                .map_err(|_| HashaiError::Integration(format!("foreign {label}; no files changed")))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), HashaiError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| HashaiError::Integration(error.to_string()))?;
    write_exact_file(path, &bytes)?;
    Ok(())
}

fn current_hash(path: &Path) -> Result<Option<String>, HashaiError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(HashaiError::Integration(
                    "managed target is unsafe".to_owned(),
                ));
            }
            Ok(Some(sha256(&fs::read(path)?)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn ensure_safe_parent(path: &Path) -> Result<(), HashaiError> {
    let parent = path
        .parent()
        .ok_or_else(|| HashaiError::ArgumentOrConfig("managed path has no parent".to_owned()))?;
    if !parent.is_absolute() || parent.to_str().is_none() {
        return Err(HashaiError::ArgumentOrConfig(
            "managed path must be absolute Unicode".to_owned(),
        ));
    }
    let mut missing = Vec::new();
    let mut cursor = parent;
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(HashaiError::Integration(
                        "managed path has an unsafe ancestor".to_owned(),
                    ));
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(cursor.to_path_buf());
                cursor = cursor.parent().ok_or_else(|| {
                    HashaiError::Integration("managed path has no safe ancestor".to_owned())
                })?;
            }
            Err(_) => {
                return Err(HashaiError::Integration(
                    "managed path ancestor is unreadable".to_owned(),
                ));
            }
        }
    }
    for directory in missing.into_iter().rev() {
        fs::create_dir(&directory)?;
        let metadata = fs::symlink_metadata(&directory)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(HashaiError::Integration(
                "managed path has an unsafe ancestor".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_existing_ancestors(path: &Path) -> Result<(), HashaiError> {
    let mut cursor = path
        .parent()
        .ok_or_else(|| HashaiError::ArgumentOrConfig("managed path has no parent".to_owned()))?;
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(HashaiError::Integration(
                        "managed path has an unsafe ancestor".to_owned(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(HashaiError::Integration(
                    "managed path ancestor is unreadable".to_owned(),
                ));
            }
        }
        match cursor.parent() {
            Some(parent) if parent != cursor => cursor = parent,
            _ => break,
        }
    }
    Ok(())
}

fn write_exact_file(path: &Path, bytes: &[u8]) -> Result<WriteOutcome, HashaiError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(HashaiError::Integration(
                "managed file is unsafe".to_owned(),
            ));
        }
        if fs::read(path)? == bytes {
            return Ok(WriteOutcome::Unchanged);
        }
    }
    ensure_safe_parent(path)?;
    let parent = path.parent().expect("checked parent");
    atomic_write(parent, path, bytes)?;
    Ok(WriteOutcome::Written)
}

fn remove_exact(file: &ManagedFile) -> Result<WriteOutcome, HashaiError> {
    let path = Path::new(&file.resolved_path);
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(HashaiError::Integration(
                    "managed file is unsafe; not removed".to_owned(),
                ));
            }
            if sha256(&fs::read(path)?) != file.sha256 {
                return Err(HashaiError::Integration(
                    "managed file is modified; not removed".to_owned(),
                ));
            }
            fs::remove_file(path)?;
            Ok(WriteOutcome::Removed)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(WriteOutcome::Absent),
        Err(error) => Err(error.into()),
    }
}

fn remove_journal_target(target: &JournalTarget) -> Result<WriteOutcome, HashaiError> {
    let path = Path::new(&target.resolved_path);
    match current_hash(path)? {
        None => Ok(WriteOutcome::Absent),
        Some(current) if Some(&current) == target.pre_sha256.as_ref() => {
            fs::remove_file(path)?;
            Ok(WriteOutcome::Removed)
        }
        Some(_) => Err(HashaiError::Integration(
            "managed transaction target changed; recovery stopped".to_owned(),
        )),
    }
}

fn supported_shells() -> [Shell; 3] {
    [Shell::Bash, Shell::Zsh, Shell::Fish]
}

fn validate_integration_shell(shell: &Shell) -> Result<(), HashaiError> {
    if matches!(shell, Shell::Auto) {
        return Err(HashaiError::ArgumentOrConfig(
            "integration shell must be bash, zsh, or fish".to_owned(),
        ));
    }
    Ok(())
}

fn ensure_regular_file(path: &Path, description: &str) -> Result<(), HashaiError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(HashaiError::Integration(format!(
            "{description} must be a regular file"
        )));
    }
    Ok(())
}

fn atomic_write(directory: &Path, destination: &Path, bytes: &[u8]) -> Result<(), HashaiError> {
    atomic_write_with(directory, destination, bytes, |file, contents| {
        file.write_all(contents)
    })
}

fn atomic_write_with(
    directory: &Path,
    destination: &Path,
    bytes: &[u8],
    write: impl FnOnce(&mut fs::File, &[u8]) -> std::io::Result<()>,
) -> Result<(), HashaiError> {
    let mut temporary = NamedTempFile::new_in(directory)?;
    write(temporary.as_file_mut(), bytes)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(destination).map_err(|_| {
        HashaiError::Integration("could not atomically publish managed file".to_owned())
    })?;
    Ok(())
}

fn artifact_version(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        line.strip_prefix(VERSION_MARKER)
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn artifact_contents(shell: &Shell, config: &Config) -> String {
    let header = format!(
        "# hashai integration artifact for {}\n# hashai-integration-version: {ARTIFACT_VERSION}\n# This file intentionally does not invoke hashai or codex during shell startup.\n",
        shell.as_str()
    );
    match shell {
        Shell::Bash => render(BASH_INTEGRATION, shell, config, &header),
        Shell::Zsh => render(ZSH_INTEGRATION, shell, config, &header),
        Shell::Fish => render(FISH_INTEGRATION, shell, config, &header),
        Shell::Auto => format!(
            "{header}# Shell editor bindings are installed by the shell-specific integration phase.\n"
        ),
    }
}

fn render(template: &str, shell: &Shell, config: &Config, header: &str) -> String {
    let trigger = match shell {
        Shell::Fish => fish_quote(&config.trigger),
        _ => sh_quote(&config.trigger),
    };
    let key = match (shell, &config.keybinding) {
        (Shell::Bash, Keybinding::CtrlG) => "\\C-g",
        (Shell::Bash, Keybinding::CtrlX) => "\\C-x",
        (Shell::Zsh, Keybinding::CtrlG) => "^G",
        (Shell::Zsh, Keybinding::CtrlX) => "^X",
        (Shell::Fish, Keybinding::CtrlG) => "\\cg",
        (Shell::Fish, Keybinding::CtrlX) => "\\cx",
        _ => "",
    };
    let rendered = match shell {
        Shell::Bash => template
            .replace(
                "__hashai_bash_trigger='# '",
                &format!("__hashai_bash_trigger={trigger}"),
            )
            .replace(
                "__hashai_bash_keybinding='\\C-g'",
                &format!("__hashai_bash_keybinding='{key}'"),
            )
            .replace(
                "__hashai_bash_enabled=1",
                &format!(
                    "__hashai_bash_enabled={}",
                    if config.trigger_enabled { 1 } else { 0 }
                ),
            ),
        Shell::Zsh => template
            .replace(
                "typeset -g __hashai_zsh_trigger='# '",
                &format!("typeset -g __hashai_zsh_trigger={trigger}"),
            )
            .replace(
                "typeset -g __hashai_zsh_keybinding='^G'",
                &format!("typeset -g __hashai_zsh_keybinding='{key}'"),
            )
            .replace(
                "typeset -g __hashai_zsh_enabled=1",
                &format!(
                    "typeset -g __hashai_zsh_enabled={}",
                    if config.trigger_enabled { 1 } else { 0 }
                ),
            ),
        Shell::Fish => template
            .replace(
                "set -g __hashai_fish_trigger '# '",
                &format!("set -g __hashai_fish_trigger {trigger}"),
            )
            .replace(
                "set -g __hashai_fish_keybinding \\cg",
                &format!("set -g __hashai_fish_keybinding {key}"),
            )
            .replace(
                "set -g __hashai_fish_enabled_config 1",
                &format!(
                    "set -g __hashai_fish_enabled_config {}",
                    if config.trigger_enabled { 1 } else { 0 }
                ),
            ),
        Shell::Auto => template.to_owned(),
    };
    format!("{header}{rendered}")
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn fish_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Write,
        path::{Path, PathBuf},
    };

    use super::{
        FileInspection, ManagedFile, OwnershipManifest, OwnershipState, atomic_write_with,
        classify_artifact, sha256,
    };
    use crate::config::Shell;

    #[test]
    fn relative_xdg_data_home_is_ignored_in_favor_of_the_platform_data_directory() {
        let fallback = PathBuf::from("/platform-data/hashai/integrations");
        assert_eq!(
            super::resolve_integration_directory(
                Some(Path::new("relative-data")),
                fallback.clone()
            ),
            fallback
        );
    }

    #[test]
    fn fault_injected_atomic_stage_write_keeps_live_bytes_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let live = temp.path().join("hashai.bash");
        let old = b"old artifact bytes";
        fs::write(&live, old).unwrap();

        let error = atomic_write_with(temp.path(), &live, b"replacement", |file, bytes| {
            file.write_all(&bytes[..3])?;
            Err(std::io::Error::other("injected target write failure"))
        })
        .unwrap_err();

        assert!(error.to_string().contains("injected target write failure"));
        assert_eq!(fs::read(&live).unwrap(), old);
    }

    #[test]
    fn ownership_classifier_covers_absent_untracked_tracked_modified_and_foreign() {
        let path = Path::new("/managed/hashai.bash");
        let expected = b"expected";
        let tracked = OwnershipManifest {
            format_version: 1,
            shell: "bash".to_owned(),
            artifact: ManagedFile {
                resolved_path: path.display().to_string(),
                sha256: sha256(expected),
            },
            loader: None,
            desired_mode: "manual".to_owned(),
        };
        let cases = [
            (
                FileInspection::Absent,
                FileInspection::Absent,
                OwnershipState::Absent,
            ),
            (
                FileInspection::Absent,
                FileInspection::Present(expected.to_vec()),
                OwnershipState::UntrackedExactExpected,
            ),
            (
                FileInspection::Absent,
                FileInspection::Present(b"foreign".to_vec()),
                OwnershipState::Foreign,
            ),
            (
                FileInspection::Present(tracked.clone()),
                FileInspection::Present(expected.to_vec()),
                OwnershipState::TrackedExact,
            ),
            (
                FileInspection::Present(tracked.clone()),
                FileInspection::Present(b"modified".to_vec()),
                OwnershipState::Modified,
            ),
            (
                FileInspection::Unsafe,
                FileInspection::Present(expected.to_vec()),
                OwnershipState::Unsafe,
            ),
            (
                FileInspection::Unreadable,
                FileInspection::Present(expected.to_vec()),
                OwnershipState::Unreadable,
            ),
        ];
        for (manifest, artifact, state) in cases {
            assert_eq!(
                classify_artifact(&Shell::Bash, path, expected, &manifest, &artifact),
                state
            );
        }
    }
}
