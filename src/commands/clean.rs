// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct CleanArgs {
    #[arg(long)]
    pub project: Option<PathBuf>,
    /// Only clean entries older than this duration (e.g. "30d", "7d", "1h").
    #[arg(long)]
    pub older_than: Option<String>,
    /// Skip confirmation prompt.
    #[arg(long)]
    pub yes: bool,
}

pub fn run(args: CleanArgs) -> Result<()> {
    let project_root = match args.project {
        Some(p) => p,
        None => crate::paths::project_root_from_cwd()?,
    };
    crate::runner::history::clean(&project_root, args.older_than.as_deref(), args.yes)
}
