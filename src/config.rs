use std::{collections::BTreeMap, fs, path::PathBuf};

use directories::ProjectDirs;
use serde::Deserialize;

use crate::HashaiError;

const ENV_TRIGGER: &str = "HASHAI_TRIGGER";
const ENV_TRIGGER_ENABLED: &str = "HASHAI_TRIGGER_ENABLED";
const ENV_KEYBINDING: &str = "HASHAI_KEYBINDING";
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
    pub fn is_supported(&self) -> bool {
        !matches!(self, Self::Auto)
    }

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
    pub trigger_enabled: bool,
    pub keybinding: Keybinding,
    pub timeout_seconds: u64,
    pub shell: Shell,
    pub codex: CodexConfig,
    pub prompt: PromptConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            trigger: "# ".to_owned(),
            trigger_enabled: true,
            keybinding: Keybinding::CtrlG,
            timeout_seconds: 30,
            shell: Shell::Auto,
            codex: CodexConfig::default(),
            prompt: PromptConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Keybinding {
    #[default]
    CtrlG,
    CtrlX,
}
impl Keybinding {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CtrlG => "ctrl-g",
            Self::CtrlX => "ctrl-x",
        }
    }
    pub fn parse(value: &str) -> Result<Self, HashaiError> {
        match value {
            "ctrl-g" => Ok(Self::CtrlG),
            "ctrl-x" => Ok(Self::CtrlX),
            _ => Err(HashaiError::ArgumentOrConfig(format!(
                "unsupported keybinding `{value}`; expected ctrl-g or ctrl-x"
            ))),
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
    pub trigger_enabled: Option<bool>,
    pub keybinding: Option<String>,
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
        if let Some(value) = environment.get(ENV_TRIGGER_ENABLED) {
            resolved.trigger_enabled = parse_bool(ENV_TRIGGER_ENABLED, value)?;
        }
        if let Some(value) = environment.get(ENV_KEYBINDING) {
            resolved.keybinding = Keybinding::parse(value)?;
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
        if let Some(value) = cli.trigger_enabled {
            resolved.trigger_enabled = value;
        }
        if let Some(value) = cli.keybinding {
            resolved.keybinding = Keybinding::parse(&value)?;
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
        let environment = relevant_environment()?;
        Self::resolve(user_config, &environment, cli)
    }
}

fn relevant_environment() -> Result<BTreeMap<String, String>, HashaiError> {
    let mut environment = BTreeMap::new();
    for name in [
        ENV_TRIGGER,
        ENV_TRIGGER_ENABLED,
        ENV_KEYBINDING,
        ENV_TIMEOUT_SECONDS,
        ENV_SHELL,
        ENV_CODEX_MODEL,
        ENV_CODEX_REASONING_EFFORT,
    ] {
        let Some(value) = std::env::var_os(name) else {
            continue;
        };
        let value = value
            .into_string()
            .map_err(|_| HashaiError::ArgumentOrConfig(format!("{name} must be valid Unicode")))?;
        environment.insert(name.to_owned(), value);
    }
    Ok(environment)
}

fn parse_bool(name: &str, value: &str) -> Result<bool, HashaiError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(HashaiError::ArgumentOrConfig(format!(
            "{name} must be exactly true or false"
        ))),
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

/// Validates a configuration supplied through either resolution or public APIs.
pub fn validate(config: &Config) -> Result<(), HashaiError> {
    let bytes = config.trigger.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 64
        || bytes.iter().any(|byte| {
            *byte == b'\r'
                || *byte == b'\n'
                || *byte == 0
                || (*byte < 0x20 && *byte != b'\t')
                || *byte == 0x7f
        })
    {
        return Err(HashaiError::ArgumentOrConfig(
            "trigger must be 1..64 bytes and contain no control characters except tab".to_owned(),
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
