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

use directories::ProjectDirs;
use tempfile::NamedTempFile;

use crate::{HashaiError, config::Shell};

pub const ARTIFACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const VERSION_MARKER: &str = "# hashai-integration-version: ";

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

#[derive(Clone, Debug)]
pub struct IntegrationManager {
    directory: PathBuf,
}

impl IntegrationManager {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn from_system() -> Result<Self, HashaiError> {
        let directories = ProjectDirs::from("com", "yasu2704", "hashai").ok_or_else(|| {
            HashaiError::ArgumentOrConfig(
                "could not determine user data directory for integrations".to_owned(),
            )
        })?;
        Ok(Self::new(directories.data_local_dir().join("integrations")))
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
        self.write_shell(&shell)
    }

    pub fn update(&self) -> Result<Vec<(Shell, WriteOutcome)>, HashaiError> {
        if !self.directory.exists() {
            return Ok(Vec::new());
        }
        self.ensure_managed_directory()?;

        let mut outcomes = Vec::new();
        for shell in supported_shells() {
            let artifact = self.artifact_path(&shell);
            if artifact.exists() || fs::symlink_metadata(&artifact).is_ok() {
                outcomes.push((shell.clone(), self.write_shell(&shell)?));
            }
        }
        Ok(outcomes)
    }

    pub fn list(&self) -> Result<Vec<InstalledIntegration>, HashaiError> {
        if !self.directory.exists() {
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

        // Make the backup durable before replacing the live artifact.  The
        // replacement is a same-directory rename, so a failed replacement
        // leaves the old live path intact.
        if let Some(existing) = existing {
            atomic_write(&self.directory, &backup, &existing)?;
        }
        atomic_write(&self.directory, &target, contents.as_bytes())?;
        Ok(WriteOutcome::Written)
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
    let mut temporary = NamedTempFile::new_in(directory)?;
    temporary.write_all(bytes)?;
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
    format!(
        "# hashai integration artifact for {}\n# hashai-integration-version: {ARTIFACT_VERSION}\n# This file intentionally does not invoke hashai or codex during shell startup.\n# Shell editor bindings are installed by the shell-specific integration phase.\n",
        shell.as_str()
    )
}
