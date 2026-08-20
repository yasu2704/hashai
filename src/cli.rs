use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "hashai",
    about = "Generate shell commands from natural language"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Generate a shell command from a natural-language request.
    Generate(GenerateArgs),
    /// Create and manage static shell integration artifacts.
    Integration(IntegrationArgs),
    /// Show the resolved non-secret configuration.
    Config(ConfigArgs),
    /// Diagnose local hashai, shell, and Codex CLI readiness.
    Doctor(DoctorArgs),
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Make one isolated Codex probe and load the selected shell's interactive startup files;
    /// this may consume network, quota, time, and run arbitrary startup side effects.
    #[arg(long)]
    pub live: bool,
    /// Output format for the diagnostic report.
    #[arg(long, default_value = "human")]
    pub format: String,
    /// Shell to diagnose: bash, zsh, or fish.
    #[arg(long)]
    pub shell: Option<String>,
}

#[derive(Debug, Args)]
pub struct GenerateArgs {
    /// Target shell: bash, zsh, or fish.
    #[arg(long)]
    pub shell: Option<String>,
    /// Override the user-configured model.
    #[arg(long)]
    pub model: Option<String>,
    /// Override the user-configured reasoning effort.
    #[arg(long)]
    pub reasoning_effort: Option<String>,
    /// Override the timeout in seconds.
    #[arg(long)]
    pub timeout_seconds: Option<u64>,
    /// Natural-language request to convert.
    pub request: String,
}

#[derive(Debug, Args)]
pub struct IntegrationArgs {
    #[command(subcommand)]
    pub command: IntegrationCommand,
}

#[derive(Debug, Subcommand)]
pub enum IntegrationCommand {
    /// Install a managed artifact. Bash and Zsh require a manual startup action.
    Install(IntegrationInstallArgs),
    /// Remove manifest-backed managed integration files.
    Uninstall(IntegrationShellArgs),
    /// Update every installed managed integration artifact.
    Update(IntegrationOverrideArgs),
    /// List installed managed integration artifacts without modifying them.
    List,
}

#[derive(Debug, Args)]
pub struct IntegrationInstallArgs {
    #[command(flatten)]
    pub overrides: IntegrationOverrideArgs,
    /// Do not install the Fish conf.d loader; print a manual source snippet.
    #[arg(long)]
    pub manual: bool,
    #[command(flatten)]
    pub target: IntegrationShellArgs,
}

#[derive(Debug, Args)]
pub struct IntegrationShellArgs {
    /// Shell to manage: bash, zsh, or fish.
    #[arg(long, required_unless_present = "shell_positional")]
    pub shell: Option<String>,
    /// Positional shell form.
    #[arg(value_name = "SHELL", conflicts_with = "shell")]
    pub shell_positional: Option<String>,
}

#[derive(Debug, Args, Default)]
pub struct IntegrationOverrideArgs {
    #[arg(long)]
    pub trigger: Option<String>,
    #[arg(long)]
    pub keybinding: Option<String>,
    #[arg(long, conflicts_with = "trigger")]
    pub disable_trigger: bool,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Show(ConfigShowArgs),
}
#[derive(Debug, Args, Default)]
pub struct ConfigShowArgs {
    #[arg(long)]
    pub trigger: Option<String>,
    #[arg(long)]
    pub keybinding: Option<String>,
    #[arg(long, conflicts_with = "trigger")]
    pub disable_trigger: bool,
}
