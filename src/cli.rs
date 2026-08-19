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
