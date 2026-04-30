// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! `a2a ask`: invoke one or more models in parallel against a single prompt.
//!
//! SPEC §3.1 / §8.

use anyhow::{Context, Result};
use clap::Args;
use std::path::PathBuf;

use crate::user_bail;

#[derive(Debug, Args)]
pub struct AskArgs {
    /// Topic slug (used as directory name under consultations/).
    pub topic: String,
    /// Path to the prompt file (markdown with optional YAML frontmatter).
    #[arg(long)]
    pub prompt_file: PathBuf,
    /// Comma-separated model aliases. When omitted, a2a runs the
    /// **first-added** alias from `~/.a2a/credentials.db`'s
    /// `model_aliases` table (SPEC §11).
    #[arg(long, value_delimiter = ',')]
    pub models: Option<Vec<String>>,
    /// Comma-separated profile chain for this run (SPEC §6.3 / §9).
    /// Order matters: first profile is the primary; subsequent ones
    /// are KeyDead fallbacks. When omitted, a2a uses a single-element
    /// chain consisting of the resolved default profile (SQLite
    /// `meta.default_profile`, else literal `"default"`, else the
    /// first registered profile). To force a single profile and
    /// disable fallback entirely, pass exactly one alias here.
    #[arg(long, value_delimiter = ',')]
    pub profiles: Option<Vec<String>>,
    /// cursor-agent's `--mode` passthrough. SPEC §8.0: clap-validated to
    /// `agent` or `plan`. When omitted, a2a uses each model alias's
    /// `default_mode` (column on the `model_aliases` row, defaults to
    /// `agent`); pass `--mode plan` here to force read-only across all.
    #[arg(long, value_parser = ["agent", "plan"])]
    pub mode: Option<String>,
    /// cursor-agent's `--sandbox <enabled|disabled>` passthrough.
    /// When omitted, a2a does not pass the flag and cursor-agent uses
    /// the user's `sandbox.json` / IDE defaults.
    #[arg(long, value_parser = ["enabled", "disabled"])]
    pub sandbox: Option<String>,
    /// Print the cursor-agent commands without running them.
    #[arg(long)]
    pub dry_run: bool,
    /// Estimate token / quota cost without running models.
    #[arg(long)]
    pub budget_only: bool,
    /// Suppress the readonly directive prepended to every prompt by
    /// default. Use when the consulted model genuinely needs to modify
    /// files in the workspace mirror.
    #[arg(long)]
    pub no_readonly_prefix: bool,
    /// Enable per-call cost audit: attach a `[models.budget]` table
    /// with char counts (token-proxy) to each successful model alias's
    /// row in `meta.toml`. Default off — turn on per-invocation when
    /// you need to audit quota burn. SPEC §14.
    #[arg(long)]
    pub log_budget: bool,
}

pub fn run(args: AskArgs) -> Result<()> {
    let prompt_file = args
        .prompt_file
        .canonicalize()
        .with_context(|| format!("prompt file not found: {}", args.prompt_file.display()))?;
    if !prompt_file.is_file() {
        user_bail!(
            "prompt file is not a regular file: {}",
            prompt_file.display()
        );
    }
    let project_root = crate::paths::project_root_from_cwd()?;
    crate::runner::ask_orchestrator(crate::runner::AskRequest {
        project_root,
        topic: args.topic,
        prompt_file,
        models: args.models,
        profiles: args.profiles,
        dry_run: args.dry_run,
        budget_only: args.budget_only,
        no_readonly_prefix: args.no_readonly_prefix,
        mode: args.mode,
        sandbox: args.sandbox,
        log_budget: args.log_budget,
    })
}
