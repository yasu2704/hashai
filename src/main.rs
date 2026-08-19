use std::{
    path::PathBuf,
    process::ExitCode as ProcessExitCode,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use clap::{Parser, error::ErrorKind};
use hashai::{
    HashaiError,
    cli::{Cli, Command},
    config::{ConfigOverrides, ConfigSources},
    platform,
    prompt::{EnvironmentInfo, PromptInput, build_prompt},
    runner::{CodexRunner, RunRequest},
};

static CANCELLED: AtomicBool = AtomicBool::new(false);

fn main() -> ProcessExitCode {
    let _ = ctrlc::set_handler(|| CANCELLED.store(true, Ordering::SeqCst));
    match run() {
        Ok(()) => ProcessExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hashai: {error}");
            ProcessExitCode::from(error.exit_code() as u8)
        }
    }
}

fn run() -> Result<(), HashaiError> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            print!("{error}");
            return Ok(());
        }
        Err(error) => return Err(HashaiError::ArgumentOrConfig(error.to_string())),
    };
    let Command::Generate(args) = cli.command;
    if args.request.trim().is_empty() {
        return Err(HashaiError::ArgumentOrConfig(
            "request must not be empty".to_owned(),
        ));
    }

    let config = ConfigSources::from_system(ConfigOverrides {
        shell: args.shell,
        model: args.model,
        reasoning_effort: args.reasoning_effort,
        timeout_seconds: args.timeout_seconds,
        ..ConfigOverrides::default()
    })?;
    let shell = config
        .shell
        .resolve(std::env::var("SHELL").ok().as_deref())?;
    let environment = detected_environment();
    platform::validate(&environment.os, shell.as_str())?;
    let current_dir = std::env::current_dir()?;
    let prompt = build_prompt(&PromptInput {
        request: args.request,
        shell,
        current_dir: current_dir.clone(),
        environment,
        extra_instructions: config.prompt.extra_instructions,
    });

    CANCELLED.store(false, Ordering::SeqCst);
    let generation = CodexRunner::new().run(
        RunRequest {
            executable: codex_executable(),
            prompt,
            current_dir,
            codex: config.codex,
            timeout: Duration::from_secs(config.timeout_seconds),
        },
        &CANCELLED,
    )?;
    println!("{}", generation.command);
    Ok(())
}

fn codex_executable() -> PathBuf {
    std::env::var_os("HASHAI_CODEX_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("codex"))
}

fn detected_environment() -> EnvironmentInfo {
    apply_test_os_override(EnvironmentInfo {
        os: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        distribution_or_version: detected_os_detail(),
    })
}

#[cfg(debug_assertions)]
fn apply_test_os_override(mut environment: EnvironmentInfo) -> EnvironmentInfo {
    if let Some(os) = std::env::var_os("HASHAI_TEST_OS").and_then(|value| value.into_string().ok())
    {
        environment.os = os;
    }
    environment
}

#[cfg(not(debug_assertions))]
fn apply_test_os_override(environment: EnvironmentInfo) -> EnvironmentInfo {
    environment
}

#[cfg(target_os = "linux")]
fn detected_os_detail() -> Option<String> {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|value| value.trim_matches('"').to_owned())
            })
        })
}

#[cfg(target_os = "macos")]
fn detected_os_detail() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn detected_os_detail() -> Option<String> {
    None
}
