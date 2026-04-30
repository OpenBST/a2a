// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! `a2a models`: manage user-global cursor model aliases.
//!
//! SPEC §11: aliases live in `~/.a2a/credentials.db`'s
//! `model_aliases` table. One set per user, shared across every
//! project on this machine; nothing is written under the project
//! tree.
//!
//! Subcommands: list / available / add / set / remove. `a2a ask`
//! without `--models` runs the **first-added** alias (lowest
//! `created_at`); pass `--models a,b,c` for any other set.

use crate::auth::store::ModelAlias;
use crate::user_bail;
use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use std::collections::BTreeSet;

#[derive(Parser, Debug)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub command: Option<ModelsCommand>,

    /// Verbose output for `list` (legacy positional flag — implies `list`).
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum ModelsCommand {
    /// Show all configured aliases (default subcommand).
    List {
        #[arg(long)]
        verbose: bool,
    },
    /// Query cursor-agent for the upstream model catalog.
    Available {
        /// Use this profile's API key (default: `auth use` profile).
        #[arg(long)]
        profile: Option<String>,
    },
    /// Register a new model alias in `~/.a2a/credentials.db`.
    Add {
        alias: String,
        #[arg(long, alias = "cursor-model")]
        model: String,
        /// SPEC §8.0: `agent` (default; cursor-agent's default mode)
        /// or `plan` (read-only).
        #[arg(long, value_parser = ["agent", "plan"])]
        mode: Option<String>,
        #[arg(long)]
        thinking_hint: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Allow overwriting an existing alias.
        #[arg(long)]
        force: bool,
    },
    /// Modify fields on an existing alias.
    Set {
        alias: String,
        #[arg(long, alias = "cursor-model")]
        model: Option<String>,
        #[arg(long, value_parser = ["agent", "plan"])]
        mode: Option<String>,
        #[arg(long)]
        thinking_hint: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Remove an alias from the user-global store.
    Remove {
        alias: String,
        #[arg(long)]
        yes: bool,
    },
}

pub fn run(args: ModelsArgs) -> Result<()> {
    match args.command {
        None => print_models_list(args.verbose),
        Some(ModelsCommand::List { verbose }) => print_models_list(verbose),
        Some(ModelsCommand::Available { profile }) => print_models_available(profile.as_deref()),
        Some(ModelsCommand::Add {
            alias,
            model,
            mode,
            thinking_hint,
            description,
            force,
        }) => cmd_add(&alias, &model, mode, thinking_hint, description, force),
        Some(ModelsCommand::Set {
            alias,
            model,
            mode,
            thinking_hint,
            description,
        }) => cmd_set(&alias, model, mode, thinking_hint, description),
        Some(ModelsCommand::Remove { alias, yes }) => cmd_remove(&alias, yes),
    }
}

fn cmd_add(
    alias: &str,
    cursor_model: &str,
    mode: Option<String>,
    thinking_hint: Option<String>,
    description: Option<String>,
    force: bool,
) -> Result<()> {
    validate_alias(alias)?;
    if cursor_model.trim().is_empty() {
        user_bail!("--model <cursor-model-id> is required and cannot be empty");
    }
    let mut store = crate::auth::store::open()?;
    let row = ModelAlias {
        alias: alias.to_string(),
        cursor_model: cursor_model.to_string(),
        default_mode: mode.unwrap_or_else(|| "agent".into()),
        thinking_hint,
        description,
        // Preserve the original `created_at` on `--force` so the
        // first-added-alias semantics (used by `a2a ask` without
        // `--models`) don't shuffle when an alias is rotated. New
        // alias inserts get the current wall-clock millisecond
        // timestamp — second-resolution would collide on rapid
        // batch inserts and degrade `list_model_aliases` ordering
        // to alphabetical (the secondary sort key).
        created_at: if force {
            store
                .get_model_alias(alias)?
                .map(|m| m.created_at)
                .unwrap_or_else(|| Utc::now().timestamp_millis())
        } else {
            Utc::now().timestamp_millis()
        },
    };
    if force {
        // Explicit overwrite: UPSERT.
        store.replace_model_alias(&row)?;
    } else {
        // Atomic insert-or-fail (no TOCTOU window): plain INSERT
        // returns SQLite ConstraintViolation when a row already
        // exists; we map that to the same UserError the prior
        // `model_alias_exists()` pre-flight produced. Two concurrent
        // `a2a models add foo` (without --force) now reliably let
        // exactly one win and the other surface a clean error,
        // instead of both UPSERT-overwriting through the race
        // window flagged in r20 review.
        match store.try_insert_model_alias(&row)? {
            crate::auth::store::TryInsertOutcome::Inserted => {}
            crate::auth::store::TryInsertOutcome::AlreadyExists => {
                user_bail!(
                    "alias '{alias}' is already registered. Use `a2a models set {alias} --model <id> \
                     [...]` to update individual fields, or pass `--force` to fully replace it."
                );
            }
        }
    }
    println!(
        "Registered alias '{alias}' (cursor_model={cursor_model}). Verify with `a2a models list`."
    );
    Ok(())
}

fn cmd_set(
    alias: &str,
    model: Option<String>,
    mode: Option<String>,
    thinking_hint: Option<String>,
    description: Option<String>,
) -> Result<()> {
    validate_alias(alias)?;
    let mut store = crate::auth::store::open()?;
    let updated = store.update_model_alias_fields(
        alias,
        model.as_deref(),
        mode.as_deref(),
        // `--thinking-hint ""` clears the column; absent flag leaves it.
        thinking_hint.as_deref().map(|s| if s.is_empty() { None } else { Some(s) }),
        description.as_deref().map(|s| if s.is_empty() { None } else { Some(s) }),
    )?;
    if !updated {
        user_bail!(
            "alias '{alias}' is not registered. Use `a2a models add {alias} --model <id>` \
             to create it."
        );
    }
    println!("Updated alias '{alias}'.");
    Ok(())
}

fn cmd_remove(alias: &str, yes: bool) -> Result<()> {
    validate_alias(alias)?;
    let mut store = crate::auth::store::open()?;
    if !store.model_alias_exists(alias)? {
        user_bail!("alias '{alias}' is not registered.");
    }
    if !yes {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!("Remove model alias '{alias}'?"))
            .default(false)
            .interact()
            .context("confirmation prompt")?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }
    store.delete_model_alias(alias)?;
    println!("Removed alias '{alias}'.");
    Ok(())
}

fn validate_alias(alias: &str) -> Result<()> {
    if alias.is_empty() {
        user_bail!("alias cannot be empty");
    }
    if !alias
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        user_bail!("alias must be alphanumeric / '-' / '_': {alias}");
    }
    Ok(())
}

// ---------- list / available ----------

fn print_models_list(verbose: bool) -> Result<()> {
    let store = crate::auth::store::open()?;
    let aliases = store.list_model_aliases()?;
    println!(
        "Configured model aliases (* = default for `a2a ask` without `--models`, total {}):",
        aliases.len()
    );
    if aliases.is_empty() {
        println!("  (none registered; run `a2a models add <alias> --model <cursor-id>`)");
        return Ok(());
    }
    for (idx, m) in aliases.iter().enumerate() {
        // SPEC §11: `a2a ask` without `--models` runs the FIRST-added
        // alias. Mark only that one — there is no manual "set
        // defaults" mechanism anymore.
        let mark = if idx == 0 { "*" } else { " " };
        let variant = ModelVariant::parse(&m.cursor_model);
        let desc = m.description.clone().unwrap_or_else(|| variant.render());
        if verbose {
            println!("{mark} {}", m.alias);
            println!("    cursor_model:   {}", m.cursor_model);
            println!("    parsed:         {desc}");
            println!("    default_mode:   {}", m.default_mode);
            if let Some(hint) = &m.thinking_hint {
                println!("    thinking_hint:  {hint}");
            }
        } else {
            println!(
                "{mark} {:<14}  {}  [{}]",
                m.alias, m.cursor_model, m.default_mode
            );
            if !desc.is_empty() {
                println!("                  {desc}");
            }
        }
    }
    println!();
    println!(
        "(`a2a ask` without `--models` runs only '{}'. Pass `--models a,b,c` for any other set.)",
        aliases[0].alias
    );
    Ok(())
}

/// Query cursor-agent for the upstream model catalog by spawning
/// `cursor-agent --list-models` with the resolved profile's API key.
/// Returns a `Vec<(model_id, description)>`.
pub fn fetch_available_models(profile_name: Option<&str>) -> Result<Vec<(String, String)>> {
    let mut store = crate::auth::store::open()?;
    let resolved_profile = match profile_name {
        Some(p) => p.to_string(),
        None => {
            // SPEC §5.3 default-profile resolution:
            //   meta.default_profile → "default" → first by created_at
            store
                .get_default_profile()
                .ok()
                .flatten()
                .or_else(|| {
                    if store.profile_exists("default").ok().unwrap_or(false) {
                        Some("default".to_string())
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    let mut profiles = store.list_profiles().ok().unwrap_or_default();
                    profiles.sort_by_key(|p| p.created_at);
                    profiles.into_iter().next().map(|p| p.name)
                })
                .unwrap_or_else(|| "default".to_string())
        }
    };
    let api_key = crate::auth::read_api_key(&mut store, &resolved_profile)
        .with_context(|| format!("read API key for profile '{resolved_profile}'"))?;

    let bin = crate::runner::cursor_agent::locate_binary()
        .ok_or_else(|| anyhow::anyhow!("cursor-agent not on PATH (run `a2a doctor`)"))?;
    let is_ps1 = bin.extension().and_then(|s| s.to_str()) == Some("ps1");
    let mut cmd = if is_ps1 {
        let mut c = std::process::Command::new("powershell.exe");
        c.arg("-NoProfile");
        c.arg("-ExecutionPolicy").arg("Bypass");
        c.arg("-File").arg(&bin);
        c
    } else {
        std::process::Command::new(&bin)
    };
    // Suppress new console window on Windows — same rationale as
    // runner::cursor_agent::no_window.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.arg("--list-models")
        .env("CURSOR_API_KEY", api_key)
        .env_remove("CURSOR_API_KEY_FILE")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    // Dedicated reader threads continuously drain stdout / stderr
    // (so the child can't block on its own write past the OS pipe
    // buffer ~4 KB on Windows / ~64 KB on Linux). Main thread polls
    // `try_wait` against the deadline; on timeout `kill()` closes
    // the pipes and the readers exit naturally.
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", bin.display()))?;
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();
    let stdout_thread = std::thread::spawn(move || -> Vec<u8> {
        use std::io::Read;
        let mut buf = Vec::new();
        if let Some(mut h) = stdout_handle {
            let _ = h.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_thread = std::thread::spawn(move || -> Vec<u8> {
        use std::io::Read;
        let mut buf = Vec::new();
        if let Some(mut h) = stderr_handle {
            let _ = h.read_to_end(&mut buf);
        }
        buf
    });
    let started = std::time::Instant::now();
    let status = loop {
        // `std::process::Child` (unlike tokio's `Command`) does NOT
        // kill on drop. If `try_wait` errors and we propagate via `?`,
        // child is dropped but the cursor-agent process survives
        // indefinitely. So: on Err, explicitly `kill()` + `wait()`
        // before propagating.
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if started.elapsed() >= TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!(
                    "cursor-agent --list-models timed out after {TIMEOUT:?}; \
                     check network / API key health"
                );
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e).context("try_wait on cursor-agent --list-models");
            }
        }
    };
    let stdout_buf = stdout_thread.join().unwrap_or_default();
    let stderr_buf = stderr_thread.join().unwrap_or_default();
    let output = std::process::Output {
        status,
        stdout: stdout_buf,
        stderr: stderr_buf,
    };
    if !output.status.success() {
        anyhow::bail!(
            "cursor-agent --list-models exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case("Available models") {
            continue;
        }
        if let Some((id, desc)) = line.split_once(" - ") {
            out.push((id.trim().to_string(), desc.trim().to_string()));
        }
    }
    Ok(out)
}

fn print_models_available(profile_name: Option<&str>) -> Result<()> {
    let store = crate::auth::store::open()?;
    let referenced: BTreeSet<String> = store
        .list_model_aliases()?
        .into_iter()
        .map(|m| m.cursor_model)
        .collect();
    let resolved = profile_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // Same fallback chain as fetch_available_models below
            // (kept inline for the purely informational pre-call
            // print): SQLite default → "default" → first profile.
            store
                .get_default_profile()
                .ok()
                .flatten()
                .or_else(|| {
                    if store.profile_exists("default").ok().unwrap_or(false) {
                        Some("default".to_string())
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    let mut profiles = store.list_profiles().ok().unwrap_or_default();
                    profiles.sort_by_key(|p| p.created_at);
                    profiles.into_iter().next().map(|p| p.name)
                })
                .unwrap_or_else(|| "default".into())
        });
    println!("Querying cursor-agent --list-models (profile={resolved}) ...");
    let entries = fetch_available_models(profile_name)?;
    println!("Found {} model(s) on this account.", entries.len());
    println!("(@ = referenced by an a2a alias on this machine)");
    println!();
    for (id, desc) in &entries {
        let mark = if referenced.contains(id) { "@" } else { " " };
        println!("{mark} {:<48}  {}", id, desc);
    }
    Ok(())
}

// ---------- model-id variant parsing (display only) ----------

/// Parsed variant tags extracted from a cursor-agent model id like
/// `claude-opus-4-7-thinking-xhigh` or `gpt-5.5-extra-high-fast`. These
/// are display-only — the source of truth is the literal `cursor_model`
/// string passed to cursor-agent.
#[derive(Debug, Clone, Default)]
struct ModelVariant {
    family: String,
    size: Option<String>,
    thinking: bool,
    speed: Option<String>,
}

impl ModelVariant {
    /// Best-effort parse of a Cursor model id. Names follow
    /// `<family>[-<size>][-thinking[-<level>]][-fast]` patterns; not all
    /// segments are present for every model.
    fn parse(cursor_model: &str) -> Self {
        let mut tokens: Vec<&str> = cursor_model.split('-').collect();
        let mut v = ModelVariant::default();

        if tokens.last() == Some(&"fast") {
            tokens.pop();
            v.speed = Some("fast".into());
        }

        // Size suffix: low / medium / high / extra-high (xhigh) / max.
        let size_match = match tokens.split_last() {
            Some((&"high", rest)) if rest.last() == Some(&"extra") => Some(("extra-high", 2usize)),
            Some((tail, _)) if matches!(*tail, "low" | "medium" | "high" | "xhigh" | "max") => {
                Some((*tail, 1usize))
            }
            _ => None,
        };
        if let Some((label, drop)) = size_match {
            for _ in 0..drop {
                tokens.pop();
            }
            v.size = Some(label.into());
        }

        if tokens.last() == Some(&"thinking") {
            tokens.pop();
            v.thinking = true;
        }

        v.family = tokens.join("-");
        v
    }

    fn render(&self) -> String {
        let mut bits: Vec<String> = Vec::new();
        if !self.family.is_empty() {
            bits.push(self.family.clone());
        }
        if let Some(sz) = &self.size {
            bits.push(format!("size={sz}"));
        }
        if self.thinking {
            bits.push("thinking".into());
        }
        if let Some(sp) = &self.speed {
            bits.push(format!("speed={sp}"));
        }
        bits.join(" / ")
    }
}
