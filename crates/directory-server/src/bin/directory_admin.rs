use std::io::Write;

use clap::{Parser, Subcommand};

/// Operator CLI for the p2p directory server — the actual "one click" purge.
#[derive(Parser)]
#[command(name = "directory-admin")]
struct Cli {
    /// Base URL of the admin listener (loopback by default on the server).
    #[arg(
        long,
        env = "DIRECTORY_ADMIN_URL",
        default_value = "http://127.0.0.1:8090"
    )]
    admin_url: String,

    /// Admin bearer token (must match DIRECTORY_ADMIN_TOKEN on the server).
    #[arg(long, env = "DIRECTORY_ADMIN_TOKEN")]
    token: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Instantly and irrecoverably wipe all directory data (users, presence, groups).
    Purge {
        /// Skip the interactive confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Purge { yes } => run_purge(&cli.admin_url, &cli.token, yes),
    }
}

fn run_purge(admin_url: &str, token: &str, yes: bool) -> anyhow::Result<()> {
    if !yes {
        eprint!(
            "This will permanently and irrecoverably delete ALL directory data \
             (users, presence, groups). Type 'purge' to continue: "
        );
        std::io::stderr().flush().ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if input.trim() != "purge" {
            eprintln!("Aborted — nothing was deleted.");
            return Ok(());
        }
    }

    let url = format!("{}/admin/purge", admin_url.trim_end_matches('/'));
    match ureq::post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
    {
        Ok(resp) => {
            println!("Purge complete (HTTP {}).", resp.status());
            Ok(())
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            anyhow::bail!("server rejected purge (HTTP {code}): {body}")
        }
        Err(e) => Err(anyhow::anyhow!(e)),
    }
}
