// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! `a2a auth` subcommands. SPEC §3.2.
//!
//! Available: add / list / use / show / remove / update.
//! Removed (SPEC §2.4 / §2.8): init / disable / enable / health / export.

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub command: AuthCommand,
}

#[derive(Subcommand, Debug)]
pub enum AuthCommand {
    /// Add a new profile.
    ///
    /// Default: interactive prompt (key not echoed). Use `--from-stdin` to
    /// feed the key from a pipe (recommended when an AI agent invokes this).
    /// SPEC §5.2: `name` is optional — if omitted, a2a takes the API key's
    /// last 6 characters (with `(1)` / `(2)` suffix on collision).
    Add {
        /// Profile name. Optional; auto-derives from key tail when absent.
        name: Option<String>,
        /// Optional human-readable note.
        #[arg(long)]
        note: Option<String>,
        /// Read API key from stdin (first non-empty line).
        #[arg(long)]
        from_stdin: bool,
    },
    /// List all profiles.
    List,
    /// Set the default profile.
    Use { name: String },
    /// Show masked key (first 4 + last 4 chars) for verification.
    Show { name: String },
    /// Remove a profile.
    Remove {
        name: String,
        /// Skip confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Update an existing profile's API key.
    Update {
        name: String,
        /// Read API key from stdin (first non-empty line).
        #[arg(long)]
        from_stdin: bool,
    },
}

pub fn run(args: AuthArgs) -> Result<()> {
    match args.command {
        AuthCommand::Add {
            name,
            note,
            from_stdin,
        } => crate::auth::cmd_add(name.as_deref(), note.as_deref(), from_stdin),
        AuthCommand::List => crate::auth::cmd_list(),
        AuthCommand::Use { name } => crate::auth::cmd_use(&name),
        AuthCommand::Show { name } => crate::auth::cmd_show(&name),
        AuthCommand::Remove { name, yes } => crate::auth::cmd_remove(&name, yes),
        AuthCommand::Update { name, from_stdin } => crate::auth::cmd_update(&name, from_stdin),
    }
}
