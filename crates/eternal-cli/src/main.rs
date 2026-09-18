use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "eternal", version, about = "Make a local audio file play forever")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Analyse a file and print its beat graph as JSON.
    Analyse { input: PathBuf },
    /// Analyse and play a file indefinitely.
    Play { input: PathBuf },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Analyse { input } => println!("{}", input.display()),
        Command::Play { input } => println!("{}", input.display()),
    }
    Ok(())
}
