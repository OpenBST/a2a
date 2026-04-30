// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! CLI subcommand definitions and dispatcher.

pub mod ask;
pub mod auth;
pub mod clean;
pub mod doctor;
pub mod init;
pub mod list;
pub mod models;
pub mod reset;
pub mod status;
pub mod welcome;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "a2a",
    version,
    about = "Multi-AI consultation bridge: parallel cursor-agent calls with profile management.",
    long_about = None,
)]
pub struct Cli {
    /// Agent-mode entry point. When set with no subcommand, prints
    /// a structured health report + imperative next-step guidance
    /// for an AI agent invoking a2a from Cursor's terminal — same
    /// situational checks as `a2a` (no args) but with **no pause**
    /// and **no Y/n prompts**, since Cursor's terminal tool spawns
    /// child processes with a TTY stdin and would deadlock on
    /// either. Ignored when a subcommand is also present.
    #[arg(long)]
    pub agent: bool,

    /// `Option<...>` so running `a2a.exe` with no arguments (terminal
    /// `a2a` or Explorer double-click on Windows) lands in the
    /// welcome wizard instead of clap's "missing subcommand" error.
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Install a2a templates (Cursor skills / rules / prompt template)
    /// into a project. Use `--path <workspace_path>` to target an
    /// explicit project root (recommended when an AI agent is driving
    /// this command); otherwise defaults to the current working
    /// directory.
    Init(init::InitArgs),
    /// Ask multiple AI models in parallel and persist their answers.
    Ask(ask::AskArgs),
    /// Manage authentication profiles (multi-account API keys).
    Auth(auth::AuthArgs),
    /// Check environment health (cursor-agent PATH, version, login state).
    Doctor(doctor::DoctorArgs),
    /// List past consultations in the current project.
    List(list::ListArgs),
    /// Clean stale consultation records and OS tempdir mirrors.
    Clean(clean::CleanArgs),
    /// Show current default profile and PATH check.
    Status(status::StatusArgs),
    /// List configured model aliases.
    Models(models::ModelsArgs),
    /// Reset model aliases or credentials to a clean state.
    Reset(reset::ResetArgs),
}

pub fn dispatch(cli: Cli) -> Result<()> {
    if cli.command.is_none() {
        return if cli.agent {
            welcome::run_agent()
        } else {
            welcome::run()
        };
    }
    // `--agent` is only meaningful as a top-level entry point. When a
    // subcommand is present, ignore the flag (clap allows the
    // combination but we don't forward it anywhere) — the subcommand
    // dispatches normally as if `--agent` weren't there.
    let Some(command) = cli.command else {
        unreachable!("guarded by `is_none` above");
    };
    match command {
        Command::Init(args) => init::run(args),
        Command::Ask(args) => ask::run(args),
        Command::Auth(args) => auth::run(args),
        Command::Doctor(args) => doctor::run(args),
        Command::List(args) => list::run(args),
        Command::Clean(args) => clean::run(args),
        Command::Status(args) => status::run(args),
        Command::Models(args) => models::run(args),
        Command::Reset(args) => reset::run(args),
    }
}
