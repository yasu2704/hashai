//! Static, versioned shell integration artifacts.
//!
//! Artifacts live only in hashai's user data directory.  This module never
//! evaluates an artifact, starts Core, or invokes Codex; shell-editor behavior
//! is deliberately left to the shell-specific Phase 2 issues.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use directories::ProjectDirs;
use fs2::FileExt;
use tempfile::NamedTempFile;

use crate::{HashaiError, config::Shell};

pub const ARTIFACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const VERSION_MARKER: &str = "# hashai-integration-version: ";
const BASH_INTEGRATION: &str = include_str!("../shell/hashai.bash");
const ZSH_INTEGRATION: &str = include_str!("../shell/hashai.zsh");

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WriteOutcome {
    Written,
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledIntegration {
    pub shell: Shell,
    pub path: PathBuf,
    pub version: Option<String>,
    pub is_current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateFailure {
    pub shell: Shell,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpdateSummary {
    pub outcomes: Vec<(Shell, WriteOutcome)>,
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
        Ok(Self::new(resolve_integration_directory(
            xdg_data_home.as_deref(),
            directories.data_local_dir().join("integrations"),
        )))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn artifact_path(&self, shell: &Shell) -> PathBuf {
        self.directory.join(format!("hashai.{}", shell.as_str()))
    }

    pub fn backup_path(&self, shell: &Shell) -> PathBuf {
        self.directory
            .join(format!("hashai.{}.bak", shell.as_str()))
    }

    pub fn generate(&self, shell: Shell) -> Result<WriteOutcome, HashaiError> {
        validate_integration_shell(&shell)?;
        self.with_write_lock(|| self.write_shell(&shell))
    }

    pub fn update(&self) -> Result<UpdateSummary, HashaiError> {
        if self.managed_directory_is_absent()? {
            return Ok(UpdateSummary::default());
        }
        self.with_write_lock(|| self.update_locked())
    }

    fn update_locked(&self) -> Result<UpdateSummary, HashaiError> {
        let mut summary = UpdateSummary::default();
        for shell in supported_shells() {
            let artifact = self.artifact_path(&shell);
            if !artifact.exists() && fs::symlink_metadata(&artifact).is_err() {
                continue;
            }
            if let Err(error) = self.preflight_update(&shell) {
                summary.failures.push(UpdateFailure {
                    shell,
                    message: error.to_string(),
                });
                continue;
            }
            match self.write_shell(&shell) {
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
        if self.managed_directory_is_absent()? {
            return Ok(Vec::new());
        }
        self.ensure_managed_directory()?;

        let mut installed = Vec::new();
        for shell in supported_shells() {
            let path = self.artifact_path(&shell);
            if !path.exists() && fs::symlink_metadata(&path).is_err() {
                continue;
            }
            ensure_regular_file(&path, "integration artifact")?;
            let contents = fs::read_to_string(&path).map_err(|error| {
                HashaiError::Integration(format!(
                    "could not read integration artifact {}: {error}",
                    path.display()
                ))
            })?;
            let version = artifact_version(&contents);
            let is_current = version.as_deref() == Some(ARTIFACT_VERSION);
            installed.push(InstalledIntegration {
                shell,
                path,
                version,
                is_current,
            });
        }
        Ok(installed)
    }

    fn write_shell(&self, shell: &Shell) -> Result<WriteOutcome, HashaiError> {
        self.ensure_managed_directory()?;
        let target = self.artifact_path(shell);
        let backup = self.backup_path(shell);
        let contents = artifact_contents(shell);

        let existing = match fs::symlink_metadata(&target) {
            Ok(_) => {
                ensure_regular_file(&target, "integration artifact")?;
                Some(fs::read(&target).map_err(|error| {
                    HashaiError::Integration(format!(
                        "could not read integration artifact {}: {error}",
                        target.display()
                    ))
                })?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if existing.as_deref() == Some(contents.as_bytes()) {
            return Ok(WriteOutcome::Unchanged);
        }

        if fs::symlink_metadata(&backup).is_ok() {
            ensure_regular_file(&backup, "integration backup")?;
        }

        // Write the backup before replacing the live artifact. The replacement
        // is a same-directory rename, so a failed replacement leaves the old
        // live path intact.
        if let Some(existing) = existing {
            atomic_write(&self.directory, &backup, &existing)?;
        }
        atomic_write(&self.directory, &target, contents.as_bytes())?;
        Ok(WriteOutcome::Written)
    }

    fn preflight_update(&self, shell: &Shell) -> Result<(), HashaiError> {
        ensure_regular_file(&self.artifact_path(shell), "integration artifact")?;
        let backup = self.backup_path(shell);
        if fs::symlink_metadata(&backup).is_ok() {
            ensure_regular_file(&backup, "integration backup")?;
        }
        Ok(())
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
        file.try_lock_exclusive().map_err(|error| {
            HashaiError::Integration(format!(
                "another integration operation is in progress ({error}); retry after it finishes"
            ))
        })?;
        operation()
    }

    fn ensure_managed_directory(&self) -> Result<(), HashaiError> {
        match fs::symlink_metadata(&self.directory) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(HashaiError::Integration(format!(
                        "managed integration directory {} must be a real directory, not a symlink or file",
                        self.directory.display()
                    )));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&self.directory)?;
                let metadata = fs::symlink_metadata(&self.directory)?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(HashaiError::Integration(format!(
                        "managed integration directory {} is unsafe",
                        self.directory.display()
                    )));
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
            Err(error) => Err(HashaiError::Integration(format!(
                "could not inspect managed integration directory {}: {error}",
                self.directory.display()
            ))),
        }
    }
}

fn resolve_integration_directory(xdg_data_home: Option<&Path>, fallback: PathBuf) -> PathBuf {
    match xdg_data_home.filter(|path| path.is_absolute()) {
        Some(data_home) => data_home.join("hashai/integrations"),
        None => fallback,
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
            "{description} {} must be a regular file, not a symlink or directory",
            path.display()
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
    temporary.persist(destination).map_err(|error| {
        HashaiError::Integration(format!(
            "could not atomically replace {}: {}",
            destination.display(),
            error.error
        ))
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

fn artifact_contents(shell: &Shell) -> String {
    let header = format!(
        "# hashai integration artifact for {}\n# hashai-integration-version: {ARTIFACT_VERSION}\n# This file intentionally does not invoke hashai or codex during shell startup.\n",
        shell.as_str()
    );
    match shell {
        Shell::Bash => format!("{header}{BASH_INTEGRATION}"),
        Shell::Zsh => format!("{header}{ZSH_INTEGRATION}"),
        Shell::Fish | Shell::Auto => format!(
            "{header}# Shell editor bindings are installed by the shell-specific integration phase.\n"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Write,
        path::{Path, PathBuf},
    };

    use super::atomic_write_with;

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
    fn fault_injected_target_write_after_backup_keeps_live_bytes_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let live = temp.path().join("hashai.bash");
        let backup = temp.path().join("hashai.bash.bak");
        let old = b"old artifact bytes";
        fs::write(&live, old).unwrap();

        super::atomic_write(temp.path(), &backup, old).unwrap();
        let error = atomic_write_with(temp.path(), &live, b"replacement", |file, bytes| {
            file.write_all(&bytes[..3])?;
            Err(std::io::Error::other("injected target write failure"))
        })
        .unwrap_err();

        assert!(error.to_string().contains("injected target write failure"));
        assert_eq!(fs::read(&live).unwrap(), old);
        assert_eq!(fs::read(&backup).unwrap(), old);
    }
}
