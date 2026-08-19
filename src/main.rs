use std::{
    path::PathBuf,
    process::ExitCode as ProcessExitCode,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use clap::{Parser, error::ErrorKind};
use hashai::{
    HashaiError,
    cli::{Cli, Command, IntegrationCommand},
    config::{ConfigOverrides, ConfigSources},
    integration::{IntegrationManager, WriteOutcome},
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
    match cli.command {
        Command::Generate(args) => run_generate(args),
        Command::Integration(args) => run_integration(args.command),
    }
}

fn run_generate(args: hashai::cli::GenerateArgs) -> Result<(), HashaiError> {
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
    let risk = hashai::risk::combine(generation.risk, hashai::risk::analyze(&generation.command));
    match risk {
        hashai::runner::Risk::Safe => {}
        hashai::runner::Risk::Review => {
            eprintln!("hashai: warning: generated command risk=review; inspect before execution");
        }
        hashai::runner::Risk::Dangerous => {
            eprintln!(
                "hashai: warning: generated command risk=dangerous; inspect carefully before execution"
            );
        }
    }
    println!("{}", generation.command);
    Ok(())
}

fn run_integration(command: IntegrationCommand) -> Result<(), HashaiError> {
    let manager = IntegrationManager::from_system()?;
    match command {
        IntegrationCommand::Generate(args) => {
            let requested_shell = args.shell.or(args.shell_positional).ok_or_else(|| {
                HashaiError::ArgumentOrConfig(
                    "integration shell must be bash, zsh, or fish".to_owned(),
                )
            })?;
            let shell = hashai::config::Shell::parse(&requested_shell)?;
            let outcome = manager.generate(shell.clone())?;
            print_write_result(&shell, manager.artifact_path(&shell), outcome);
        }
        IntegrationCommand::Update => {
            let summary = manager.update()?;
            for (shell, outcome) in summary.outcomes {
                print_write_result(&shell, manager.artifact_path(&shell), outcome);
            }
            if !summary.failures.is_empty() {
                for failure in &summary.failures {
                    eprintln!(
                        "hashai: integration update for {} failed: {}",
                        failure.shell.as_str(),
                        failure.message
                    );
                }
                return Err(HashaiError::Integration(format!(
                    "{} integration update(s) failed",
                    summary.failures.len()
                )));
            }
        }
        IntegrationCommand::List => {
            for installed in manager.list()? {
                let version = installed.version.as_deref().unwrap_or("unknown");
                let status = if installed.is_current {
                    "current"
                } else {
                    "outdated"
                };
                println!(
                    "{}\t{}\t{}\t{}",
                    installed.shell.as_str(),
                    version,
                    status,
                    installed.path.display()
                );
            }
        }
    }
    Ok(())
}

fn print_write_result(shell: &hashai::config::Shell, path: PathBuf, outcome: WriteOutcome) {
    let action = match outcome {
        WriteOutcome::Written => "written",
        WriteOutcome::Unchanged => "unchanged",
    };
    println!("{}\t{}\t{}", shell.as_str(), action, path.display());
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
