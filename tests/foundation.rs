use std::{collections::BTreeMap, path::Path};

use assert_cmd::Command as AssertCommand;
use clap::Parser;
use hashai::{
    ExitCode, HashaiError,
    cli::{Cli, Command},
    config::{Config, ConfigOverrides, ConfigSources, Shell},
    prompt::{EnvironmentInfo, PromptInput, build_prompt},
};

#[test]
fn ac1_binary_builds_and_exposes_generate() {
    AssertCommand::cargo_bin("hashai")
        .unwrap()
        .args(["generate", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Generate a shell command"));
}

#[test]
fn ac2_generate_parses_every_supported_shell() {
    for shell in ["bash", "zsh", "fish"] {
        let cli = Cli::try_parse_from(["hashai", "generate", "--shell", shell, "find large files"])
            .unwrap();
        assert!(matches!(cli.command, Command::Generate(_)));
    }
}

#[test]
fn ac3_precedence_is_cli_then_environment_then_user_config_then_defaults() {
    let user = Config {
        trigger: ",,".to_owned(),
        timeout_seconds: 11,
        shell: Shell::Zsh,
        codex: hashai::config::CodexConfig {
            model: "user-model".to_owned(),
            reasoning_effort: "minimal".to_owned(),
        },
        prompt: hashai::config::PromptConfig {
            extra_instructions: Some("user instructions".to_owned()),
        },
    };
    let env = BTreeMap::from([
        ("HASHAI_TRIGGER".to_owned(), "## ".to_owned()),
        ("HASHAI_TIMEOUT_SECONDS".to_owned(), "22".to_owned()),
        ("HASHAI_SHELL".to_owned(), "fish".to_owned()),
        (
            "HASHAI_CODEX_MODEL".to_owned(),
            "environment-model".to_owned(),
        ),
        (
            "HASHAI_CODEX_REASONING_EFFORT".to_owned(),
            "medium".to_owned(),
        ),
    ]);
    let cli = ConfigOverrides {
        trigger: Some("# ".to_owned()),
        timeout_seconds: Some(33),
        shell: Some("bash".to_owned()),
        model: Some("cli-model".to_owned()),
        reasoning_effort: Some("high".to_owned()),
    };

    let resolved = ConfigSources::resolve(Some(user), &env, cli).unwrap();

    assert_eq!(resolved.trigger, "# ");
    assert_eq!(resolved.timeout_seconds, 33);
    assert_eq!(resolved.shell, Shell::Bash);
    assert_eq!(resolved.codex.model, "cli-model");
    assert_eq!(resolved.codex.reasoning_effort, "high");
    assert_eq!(
        resolved.prompt.extra_instructions.as_deref(),
        Some("user instructions")
    );
}

#[test]
fn ac4_defaults_match_the_design() {
    let defaults = Config::default();
    assert_eq!(defaults.codex.model, "gpt-5.6-luna");
    assert_eq!(defaults.codex.reasoning_effort, "low");
    assert_eq!(defaults.timeout_seconds, 30);
}

#[test]
fn ac5_prompt_contains_only_the_requested_minimal_context() {
    let input = PromptInput {
        request: "現在のディレクトリを一覧にして".to_owned(),
        shell: Shell::Fish,
        current_dir: Path::new("/work/project").to_path_buf(),
        environment: EnvironmentInfo {
            os: "linux".to_owned(),
            architecture: "x86_64".to_owned(),
            distribution_or_version: Some("Ubuntu 24.04".to_owned()),
        },
        extra_instructions: Some("Prefer rg over grep when available.".to_owned()),
    };

    let prompt = build_prompt(&input);

    assert!(prompt.contains("Target operating system: linux (Ubuntu 24.04)"));
    assert!(prompt.contains("Target shell: fish"));
    assert!(prompt.contains("Current working directory: /work/project"));
    assert!(prompt.contains("User request: 現在のディレクトリを一覧にして"));
    assert!(prompt.contains("Do not execute commands"));
    assert!(prompt.contains("Do not use sudo unless explicitly necessary"));
    assert!(prompt.contains("Do not use Bash syntax for Fish"));
    assert!(prompt.contains("Prefer rg over grep when available."));
    assert!(!prompt.contains("HOME="));
    assert!(!prompt.contains("GITHUB_TOKEN"));
}

#[test]
fn ac6_config_path_is_user_scoped_and_not_project_scoped() {
    let path = hashai::config::user_config_path().unwrap();
    assert!(path.ends_with("hashai/config.toml"));
    assert!(!path.starts_with("."));
}

#[test]
fn ac6_generate_ignores_a_project_local_config_file() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("config.toml"),
        "this is not valid TOML =",
    )
    .unwrap();
    let config_home = tempfile::tempdir().unwrap();

    AssertCommand::cargo_bin("hashai")
        .unwrap()
        .current_dir(project.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .env_remove("HASHAI_TRIGGER")
        .env_remove("HASHAI_TIMEOUT_SECONDS")
        .env_remove("HASHAI_SHELL")
        .env_remove("HASHAI_CODEX_MODEL")
        .env_remove("HASHAI_CODEX_REASONING_EFFORT")
        .args(["generate", "--shell", "bash", "list files"])
        .assert()
        .success();
}

#[test]
fn ac7_argument_errors_and_unsupported_operating_systems_have_distinct_exit_codes() {
    let unsupported_os = hashai::platform::validate("windows", "bash").unwrap_err();
    let unsupported_shell = Shell::parse("powershell").unwrap_err();

    assert_eq!(
        unsupported_os.exit_code(),
        ExitCode::UnsupportedPlatform as i32
    );
    assert_eq!(
        unsupported_shell.exit_code(),
        ExitCode::ArgumentOrConfig as i32
    );
    assert!(matches!(
        unsupported_os,
        HashaiError::UnsupportedPlatform(_)
    ));
}

#[test]
fn ac7_empty_requests_and_unknown_shells_are_argument_errors() {
    AssertCommand::cargo_bin("hashai")
        .unwrap()
        .args(["generate", "--shell", "powershell", "list files"])
        .assert()
        .code(ExitCode::ArgumentOrConfig as i32);
    AssertCommand::cargo_bin("hashai")
        .unwrap()
        .args(["generate", "   "])
        .assert()
        .code(ExitCode::ArgumentOrConfig as i32);
}
