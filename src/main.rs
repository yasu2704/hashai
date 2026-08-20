use std::{
    path::PathBuf,
    process::ExitCode as ProcessExitCode,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use clap::{Parser, error::ErrorKind};
use hashai::{
    HashaiError,
    cli::{Cli, Command, ConfigCommand, DoctorArgs, IntegrationCommand, IntegrationOverrideArgs},
    config::{ConfigOverrides, ConfigSources},
    integration::{IntegrationManager, WriteOutcome},
    platform,
    prompt::{EnvironmentInfo, PromptInput, build_prompt},
    runner::{CodexRunner, RunRequest},
};

static CANCELLED: AtomicBool = AtomicBool::new(false);

fn main() -> ProcessExitCode {
    let _terminal_foreground = match TerminalForegroundGuard::from_environment() {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("hashai: could not acquire terminal foreground: {error}");
            return ProcessExitCode::from(1);
        }
    };
    let _ = ctrlc::set_handler(|| CANCELLED.store(true, Ordering::SeqCst));
    match run() {
        Ok(()) => ProcessExitCode::SUCCESS,
        Err(HashaiError::Diagnostic(exit_code)) => ProcessExitCode::from(exit_code as u8),
        Err(error) => {
            eprintln!("hashai: {error}");
            ProcessExitCode::from(error.exit_code() as u8)
        }
    }
}

struct TerminalForegroundGuard {
    terminal_fd: libc::c_int,
    original_group: libc::pid_t,
}

impl TerminalForegroundGuard {
    fn from_environment() -> std::io::Result<Option<Self>> {
        if std::env::var_os("HASHAI_BASH_FOREGROUND_HANDOFF").as_deref()
            != Some(std::ffi::OsStr::new("1"))
        {
            return Ok(None);
        }
        let terminal_fd = libc::STDIN_FILENO;
        let original_group = unsafe { libc::tcgetpgrp(terminal_fd) };
        if original_group == -1 {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { libc::setpgid(0, 0) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
        set_terminal_group(terminal_fd, unsafe { libc::getpgrp() })?;
        Ok(Some(Self {
            terminal_fd,
            original_group,
        }))
    }
}

impl Drop for TerminalForegroundGuard {
    fn drop(&mut self) {
        let _ = set_terminal_group(self.terminal_fd, self.original_group);
    }
}

fn set_terminal_group(fd: libc::c_int, group: libc::pid_t) -> std::io::Result<()> {
    let mut blocked = unsafe { std::mem::zeroed::<libc::sigset_t>() };
    let mut previous = unsafe { std::mem::zeroed::<libc::sigset_t>() };
    unsafe {
        libc::sigemptyset(&mut blocked);
        libc::sigaddset(&mut blocked, libc::SIGTTOU);
        let mask_status = libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut previous);
        if mask_status != 0 {
            return Err(std::io::Error::from_raw_os_error(mask_status));
        }
    }
    let result = unsafe { libc::tcsetpgrp(fd, group) };
    let error = (result == -1).then(std::io::Error::last_os_error);
    unsafe {
        libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
    }
    match error {
        Some(error) => Err(error),
        None => Ok(()),
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
        Command::Config(args) => match args.command {
            ConfigCommand::Show(args) => run_config_show(args),
        },
        Command::Doctor(args) => run_doctor(args),
    }
}

fn run_doctor(args: DoctorArgs) -> Result<(), HashaiError> {
    if !matches!(args.format.as_str(), "human" | "json") {
        return Err(HashaiError::ArgumentOrConfig(
            "doctor format must be human or json".to_owned(),
        ));
    }
    let config = ConfigSources::from_system(ConfigOverrides::default())?;
    let report = hashai::doctor::run(
        &config,
        args.shell.as_deref(),
        args.live,
        &detected_environment().os,
        &CANCELLED,
    );
    if args.format == "json" {
        println!(
            "{}",
            serde_json::to_string(&report).expect("doctor report is serializable")
        );
    } else {
        for check in &report.checks {
            println!(
                "{:4} {:32} {}",
                check.status.as_str(),
                check.id,
                check.message
            );
        }
        println!(
            "overall: {} (exit {})",
            report.overall.as_str(),
            report.exit
        );
    }
    if report.exit == 0 {
        Ok(())
    } else {
        Err(HashaiError::Diagnostic(report.exit))
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

fn overrides(args: IntegrationOverrideArgs) -> ConfigOverrides {
    ConfigOverrides {
        trigger: args.trigger,
        keybinding: args.keybinding,
        trigger_enabled: args.disable_trigger.then_some(false),
        ..ConfigOverrides::default()
    }
}

fn run_config_show(args: hashai::cli::ConfigShowArgs) -> Result<(), HashaiError> {
    let config = ConfigSources::from_system(ConfigOverrides {
        trigger: args.trigger,
        keybinding: args.keybinding,
        trigger_enabled: args.disable_trigger.then_some(false),
        ..ConfigOverrides::default()
    })?;
    println!(
        "trigger = {:?}\ntrigger_enabled = {}\nkeybinding = {:?}\nprompt.extra_instructions = {}",
        config.trigger,
        config.trigger_enabled,
        config.keybinding.as_str(),
        if config.prompt.extra_instructions.is_some() {
            "\"<set>\""
        } else {
            "\"<unset>\""
        }
    );
    Ok(())
}

fn run_integration(command: IntegrationCommand) -> Result<(), HashaiError> {
    let manager = IntegrationManager::from_system()?;
    match command {
        IntegrationCommand::Install(args) => {
            let requested_shell = args
                .target
                .shell
                .or(args.target.shell_positional)
                .ok_or_else(|| {
                    HashaiError::ArgumentOrConfig(
                        "integration shell must be bash, zsh, or fish".to_owned(),
                    )
                })?;
            let shell = hashai::config::Shell::parse(&requested_shell)?;
            let config = ConfigSources::from_system(overrides(args.overrides))?;
            let manual = args.manual || shell != hashai::config::Shell::Fish;
            let outcome = match manager.install_with_config(shell.clone(), &config, manual) {
                Ok(outcome) => outcome,
                Err(error) => {
                    print_failed_component_records(&shell, &error);
                    return Err(error);
                }
            };
            println!(
                "{}\tartifact\t{}",
                shell.as_str(),
                outcome_name(&outcome.artifact)
            );
            println!(
                "{}\tloader\t{}",
                shell.as_str(),
                outcome
                    .loader
                    .as_ref()
                    .map(outcome_name)
                    .unwrap_or("not-attempted")
            );
            println!(
                "{}\tmanifest\t{}",
                shell.as_str(),
                outcome_name(&outcome.manifest)
            );
            if manual {
                println!("{}\tstartup\tmanual-action-required", shell.as_str());
                println!("# hashai snippet begin");
                println!("source {}", shell_quote(&outcome.artifact_path));
                println!("# hashai snippet end");
            }
        }
        IntegrationCommand::Uninstall(args) => {
            let requested_shell = args.shell.or(args.shell_positional).ok_or_else(|| {
                HashaiError::ArgumentOrConfig(
                    "integration shell must be bash, zsh, or fish".to_owned(),
                )
            })?;
            let shell = hashai::config::Shell::parse(&requested_shell)?;
            let outcome = match manager.uninstall(shell.clone()) {
                Ok(outcome) => outcome,
                Err(error) => {
                    print_failed_component_records(&shell, &error);
                    return Err(error);
                }
            };
            println!(
                "{}\tartifact\t{}",
                shell.as_str(),
                outcome_name(&outcome.artifact)
            );
            println!(
                "{}\tloader\t{}",
                shell.as_str(),
                outcome
                    .loader
                    .as_ref()
                    .map(outcome_name)
                    .unwrap_or("not-attempted")
            );
            println!(
                "{}\tmanifest\t{}",
                shell.as_str(),
                outcome_name(&outcome.manifest)
            );
        }
        IntegrationCommand::Update(args) => {
            let config = ConfigSources::from_system(overrides(args))?;
            let summary = manager.update_with_config(&config)?;
            for (shell, outcome) in summary.outcomes {
                println!(
                    "{}\tartifact\t{}",
                    shell.as_str(),
                    outcome_name(&outcome.artifact)
                );
                println!(
                    "{}\tloader\t{}",
                    shell.as_str(),
                    outcome
                        .loader
                        .as_ref()
                        .map(outcome_name)
                        .unwrap_or("not-attempted")
                );
                println!(
                    "{}\tmanifest\t{}",
                    shell.as_str(),
                    outcome_name(&outcome.manifest)
                );
            }
            if !summary.failures.is_empty() {
                for failure in &summary.failures {
                    let status = failure_status(&failure.message);
                    println!("{}\tartifact\t{}", failure.shell.as_str(), status);
                    println!("{}\tloader\tnot-attempted", failure.shell.as_str());
                    println!("{}\tmanifest\tnot-attempted", failure.shell.as_str());
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
            let config = ConfigSources::from_system(ConfigOverrides::default())?;
            for installed in manager.list_with_config(&config)? {
                let version = installed.version.as_deref().unwrap_or("unknown");
                let status = match installed.state {
                    hashai::integration::OwnershipState::TrackedExact => "current",
                    hashai::integration::OwnershipState::TrackedPrior => "prior",
                    hashai::integration::OwnershipState::UntrackedExactExpected => {
                        "untracked-expected"
                    }
                    hashai::integration::OwnershipState::Modified => "modified",
                    hashai::integration::OwnershipState::Foreign => "foreign",
                    hashai::integration::OwnershipState::Unsafe => "unsafe",
                    hashai::integration::OwnershipState::Unreadable => "unreadable",
                    hashai::integration::OwnershipState::InterruptedRecoverable => {
                        "interrupted-recoverable"
                    }
                    hashai::integration::OwnershipState::Absent => "absent",
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

fn outcome_name(outcome: &WriteOutcome) -> &'static str {
    match outcome {
        WriteOutcome::Written => "written",
        WriteOutcome::Unchanged => "unchanged",
        WriteOutcome::Adopted => "adopted",
        WriteOutcome::Removed => "removed",
        WriteOutcome::Absent => "absent",
        WriteOutcome::ManualActionRequired => "manual-action-required",
    }
}

fn shell_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn failure_status(message: &str) -> &'static str {
    if message.contains("unsafe") || message.contains("symlink") {
        "unsafe"
    } else if message.contains("unreadable") {
        "unreadable"
    } else if message.contains("journal") || message.contains("interrupted") {
        "interrupted-recoverable"
    } else {
        "conflict"
    }
}

fn print_failed_component_records(shell: &hashai::config::Shell, error: &HashaiError) {
    println!(
        "{}\tartifact\t{}",
        shell.as_str(),
        failure_status(&error.to_string())
    );
    println!("{}\tloader\tnot-attempted", shell.as_str());
    println!("{}\tmanifest\tnot-attempted", shell.as_str());
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
