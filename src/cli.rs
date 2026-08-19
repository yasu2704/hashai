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
    /// Generate a managed integration artifact for one shell.
    Generate(IntegrationGenerateArgs),
    /// Update every installed managed integration artifact.
    Update,
    /// List installed managed integration artifacts without modifying them.
    List,
}

#[derive(Debug, Args)]
pub struct IntegrationGenerateArgs {
    /// Shell to generate: bash, zsh, or fish.
    #[arg(long, required_unless_present = "shell_positional")]
    pub shell: Option<String>,
    /// Backwards-compatible positional form shown in the design document.
    #[arg(value_name = "SHELL", conflicts_with = "shell")]
    pub shell_positional: Option<String>,
}
