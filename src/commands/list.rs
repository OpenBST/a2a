// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct ListArgs {
    #[arg(long)]
    pub project: Option<PathBuf>,
}

pub fn run(args: ListArgs) -> Result<()> {
    let project_root = match args.project {
        Some(p) => p,
        None => crate::paths::project_root_from_cwd()?,
    };
    crate::runner::history::list(&project_root)
}
