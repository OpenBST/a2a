// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! `a2a reset`: clear user-global state to a known baseline.
//!
//! SPEC §11: the only persistent state a2a owns is
//! `~/.a2a/credentials.db` (profiles + meta + model_aliases).
//! `a2a reset` exposes two operations:
//!   - `reset models` : wipe the SQLite `model_aliases` table.
//!   - `reset credentials` : delete `~/.a2a/credentials.db` entirely.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct ResetArgs {
    #[command(subcommand)]
    pub command: ResetCommand,
}

#[derive(Subcommand, Debug)]
pub enum ResetCommand {
    /// Remove all user-global model aliases from the SQLite store.
    /// Profiles and `meta.default_profile` are left untouched.
    Models {
        #[arg(long)]
        yes: bool,
    },
    /// Delete `~/.a2a/credentials.db` entirely (profiles + meta +
    /// model_aliases). Irreversible.
    Credentials {
        #[arg(long)]
        yes: bool,
    },
}

pub fn run(args: ResetArgs) -> Result<()> {
    match args.command {
        ResetCommand::Models { yes } => reset_models(yes),
        ResetCommand::Credentials { yes } => reset_credentials(yes),
    }
}

fn confirm(prompt: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    let confirmed = dialoguer::Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .context("confirmation prompt")?;
    Ok(confirmed)
}

fn reset_models(yes: bool) -> Result<()> {
    let mut store = crate::auth::store::open()?;
    let count = store.list_model_aliases()?.len();
    if count == 0 {
        println!("No model aliases registered; nothing to remove.");
        return Ok(());
    }
    if !confirm(
        &format!("Remove {count} model alias(es) from ~/.a2a/credentials.db?"),
        yes,
    )? {
        println!("Aborted.");
        return Ok(());
    }
    let removed = store.delete_all_model_aliases()?;
    println!("Removed {removed} model alias(es).");
    println!(
        "Run `a2a models add <alias> --model <cursor-id>` to register new aliases. \
         (Profiles and default profile are unchanged.)"
    );
    Ok(())
}

fn reset_credentials(yes: bool) -> Result<()> {
    let db_path = crate::paths::credentials_db_path()?;
    if !db_path.exists() {
        println!("Credential database does not exist; nothing to reset.");
        return Ok(());
    }
    if !confirm(
        &format!(
            "Delete {} and remove ALL stored API keys + model aliases? This cannot be undone.",
            db_path.display()
        ),
        yes,
    )? {
        println!("Aborted.");
        return Ok(());
    }
    std::fs::remove_file(&db_path).with_context(|| format!("delete {}", db_path.display()))?;
    println!(
        "Credential database deleted. Run `a2a auth add` to register an API key, \
         then `a2a models add <alias> --model <cursor-id>` to register an alias."
    );
    Ok(())
}
