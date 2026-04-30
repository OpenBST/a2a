// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct InitArgs {
    /// Project root directory. Defaults to the current working
    /// directory when omitted.
    ///
    /// AI agents driving `a2a init` from Cursor's terminal should
    /// always pass `--path <absolute_workspace_path>` explicitly,
    /// since the agent's terminal cwd may not match the user's
    /// project root.
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Overwrite existing live template files (`.cursor/skills/...`,
    /// etc.). The staged copies under `<project>/.a2a/template/`
    /// are always overwritten — they're a per-project audit log
    /// that should reflect what the current `a2a` binary embeds.
    #[arg(long)]
    pub force: bool,
}

pub fn run(args: InitArgs) -> Result<()> {
    // SPEC §3: `init` initialises THIS directory, not the nearest
    // parent project. Don't `project_root_from_cwd` here — that would
    // walk up to a parent's `.a2a/` and re-init the parent (with
    // `--force` it would *overwrite* parent templates), surprising
    // users who created a new sub-project intentionally.
    let project_root = match args.path {
        Some(p) => p,
        None => std::env::current_dir().context("failed to detect cwd")?,
    };
    let project_root = project_root
        .canonicalize()
        .with_context(|| format!("project root does not exist: {}", project_root.display()))?;

    crate::paths::install_templates_into_project(&project_root, args.force)
}
