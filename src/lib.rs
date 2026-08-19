//! Side-effect-free foundations for the hashai command generator.

pub mod cli;
pub mod config;
pub mod doctor;
pub mod integration;
pub mod platform;
pub mod prompt;
pub mod risk;
pub mod runner;

#[cfg(test)]
mod integration_artifacts_tests;

use std::fmt;

/// Process exit codes documented in `docs/design.md` section 11.1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    General = 1,
    ArgumentOrConfig = 2,
    CodexNotFound = 3,
    Unauthenticated = 4,
    ModelUnavailable = 5,
    Timeout = 6,
    Cancelled = 7,
    InvalidOutput = 8,
    UnsupportedPlatform = 9,
}

#[derive(Debug)]
pub enum HashaiError {
    ArgumentOrConfig(String),
    UnsupportedPlatform(String),
    CodexNotFound(String),
    Unauthenticated(String),
    ModelUnavailable(String),
    Timeout(String),
    Cancelled(String),
    InvalidOutput(String),
    Integration(String),
    Diagnostic(i32),
    Io(std::io::Error),
}

impl HashaiError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::ArgumentOrConfig(_) => ExitCode::ArgumentOrConfig as i32,
            Self::UnsupportedPlatform(_) => ExitCode::UnsupportedPlatform as i32,
            Self::CodexNotFound(_) => ExitCode::CodexNotFound as i32,
            Self::Unauthenticated(_) => ExitCode::Unauthenticated as i32,
            Self::ModelUnavailable(_) => ExitCode::ModelUnavailable as i32,
            Self::Timeout(_) => ExitCode::Timeout as i32,
            Self::Cancelled(_) => ExitCode::Cancelled as i32,
            Self::InvalidOutput(_) => ExitCode::InvalidOutput as i32,
            Self::Integration(_) => ExitCode::General as i32,
            Self::Diagnostic(exit_code) => *exit_code,
            Self::Io(_) => ExitCode::General as i32,
        }
    }
}

impl fmt::Display for HashaiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArgumentOrConfig(message)
            | Self::UnsupportedPlatform(message)
            | Self::CodexNotFound(message)
            | Self::Unauthenticated(message)
            | Self::ModelUnavailable(message)
            | Self::Timeout(message)
            | Self::Cancelled(message)
            | Self::InvalidOutput(message) => formatter.write_str(message),
            Self::Integration(message) => formatter.write_str(message),
            Self::Diagnostic(_) => Ok(()),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
        }
    }
}

impl std::error::Error for HashaiError {}

impl From<std::io::Error> for HashaiError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
