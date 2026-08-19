use std::{collections::BTreeMap, fs, path::PathBuf};

use directories::ProjectDirs;
use serde::Deserialize;

use crate::HashaiError;

const ENV_TRIGGER: &str = "HASHAI_TRIGGER";
const ENV_TIMEOUT_SECONDS: &str = "HASHAI_TIMEOUT_SECONDS";
const ENV_SHELL: &str = "HASHAI_SHELL";
const ENV_CODEX_MODEL: &str = "HASHAI_CODEX_MODEL";
const ENV_CODEX_REASONING_EFFORT: &str = "HASHAI_CODEX_REASONING_EFFORT";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Shell {
    Auto,
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    pub fn parse(value: &str) -> Result<Self, HashaiError> {
        match value {
            "auto" => Ok(Self::Auto),
            "bash" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "fish" => Ok(Self::Fish),
            _ => Err(HashaiError::ArgumentOrConfig(format!(
                "unsupported shell `{value}`; expected bash, zsh, or fish"
            ))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
        }
    }

    pub fn resolve(&self, shell_environment: Option<&str>) -> Result<Self, HashaiError> {
        if *self != Self::Auto {
            return Ok(self.clone());
        }

        let detected = shell_environment
            .and_then(|value| std::path::Path::new(value).file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("bash");
        Self::parse(detected)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub trigger: String,
    pub timeout_seconds: u64,
    pub shell: Shell,
    pub codex: CodexConfig,
    pub prompt: PromptConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            trigger: "# ".to_owned(),
            timeout_seconds: 30,
            shell: Shell::Auto,
            codex: CodexConfig::default(),
            prompt: PromptConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CodexConfig {
    pub model: String,
    pub reasoning_effort: String,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            model: "gpt-5.6-luna".to_owned(),
            reasoning_effort: "low".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PromptConfig {
    /// User configuration only; environment variables cannot inject prompt text.
    pub extra_instructions: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ConfigOverrides {
    pub trigger: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub shell: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

pub struct ConfigSources;

impl ConfigSources {
    /// Resolves defaults < user config < environment < CLI.
    pub fn resolve(
        user_config: Option<Config>,
        environment: &BTreeMap<String, String>,
        cli: ConfigOverrides,
    ) -> Result<Config, HashaiError> {
        let mut resolved = user_config.unwrap_or_default();

        if let Some(value) = environment.get(ENV_TRIGGER) {
            resolved.trigger = value.clone();
        }
        if let Some(value) = environment.get(ENV_TIMEOUT_SECONDS) {
            resolved.timeout_seconds = value.parse().map_err(|_| {
                HashaiError::ArgumentOrConfig(format!("{ENV_TIMEOUT_SECONDS} must be an integer"))
            })?;
        }
        if let Some(value) = environment.get(ENV_SHELL) {
            resolved.shell = Shell::parse(value)?;
        }
        if let Some(value) = environment.get(ENV_CODEX_MODEL) {
            resolved.codex.model = value.clone();
        }
        if let Some(value) = environment.get(ENV_CODEX_REASONING_EFFORT) {
            resolved.codex.reasoning_effort = value.clone();
        }

        if let Some(value) = cli.trigger {
            resolved.trigger = value;
        }
        if let Some(value) = cli.timeout_seconds {
            resolved.timeout_seconds = value;
        }
        if let Some(value) = cli.shell {
            resolved.shell = Shell::parse(&value)?;
        }
        if let Some(value) = cli.model {
            resolved.codex.model = value;
        }
        if let Some(value) = cli.reasoning_effort {
            resolved.codex.reasoning_effort = value;
        }

        validate(&resolved)?;
        Ok(resolved)
    }

    /// Reads only the user's configuration path.  No repository-local path is considered.
    pub fn from_system(cli: ConfigOverrides) -> Result<Config, HashaiError> {
        let user_config = load_user_config()?;
        let environment = std::env::vars().collect();
        Self::resolve(user_config, &environment, cli)
    }
}

pub fn user_config_path() -> Result<PathBuf, HashaiError> {
    ProjectDirs::from("com", "yasu2704", "hashai")
        .map(|directories| directories.config_dir().join("config.toml"))
        .ok_or_else(|| {
            HashaiError::ArgumentOrConfig("could not determine user config directory".to_owned())
        })
}

fn load_user_config() -> Result<Option<Config>, HashaiError> {
    let path = user_config_path()?;
    match fs::read_to_string(&path) {
        Ok(contents) => toml::from_str(&contents).map(Some).map_err(|error| {
            HashaiError::ArgumentOrConfig(format!(
                "invalid user config {}: {error}",
                path.display()
            ))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn validate(config: &Config) -> Result<(), HashaiError> {
    if config.trigger.is_empty() {
        return Err(HashaiError::ArgumentOrConfig(
            "trigger must not be empty".to_owned(),
        ));
    }
    if config.timeout_seconds == 0 {
        return Err(HashaiError::ArgumentOrConfig(
            "timeout_seconds must be greater than zero".to_owned(),
        ));
    }
    if config.codex.model.trim().is_empty() || config.codex.reasoning_effort.trim().is_empty() {
        return Err(HashaiError::ArgumentOrConfig(
            "Codex model and reasoning effort must not be empty".to_owned(),
        ));
    }
    Ok(())
}
