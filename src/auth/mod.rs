// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! Plaintext profile management. SPEC §3.2 / §4 / §5.
//!
//! No encryption (SPEC §2.1), no `disabled_until` state machine
//! (SPEC §2.4), no hash dedup (SPEC §2.3), no `auth export` (SPEC §2.8).
//!
//! Subcommands: add / list / use / show / remove / update.
//! `auth init` / `disable` / `enable` / `health` / `export` are gone.

pub mod store;

pub use store::Profile;

use crate::user_bail;
use anyhow::{Context, Result};
use chrono::Utc;

/// Add a profile. SPEC §5.2: `name` may be `None` — in that case
/// a2a uses the API key's last 6 characters as the name, with `(1)`,
/// `(2)`, ... suffix on collision.
pub fn cmd_add(name: Option<&str>, note: Option<&str>, from_stdin: bool) -> Result<()> {
    let key = if from_stdin {
        read_first_stdin_line().context("read API key from stdin")?
    } else {
        prompt_api_key()?
    };
    if key.is_empty() {
        user_bail!("empty API key");
    }
    if !looks_like_cursor_key(&key) {
        println!(
            "warning: key does not match expected Cursor token prefixes (key_ / crsr_); accepting anyway"
        );
    }

    let mut db = store::open()?;

    let resolved_name = match name {
        Some(n) => {
            validate_profile_name(n)?;
            if db.profile_exists(n)? {
                user_bail!(
                    "profile name '{n}' is already in use. To overwrite the stored key, \
                     run `a2a auth update {n} --from-stdin`. To keep the existing profile \
                     and store the new key separately, re-run with a different name."
                );
            }
            n.to_string()
        }
        None => auto_pick_name(&db, &key)?,
    };

    db.insert_profile(&Profile {
        name: resolved_name.clone(),
        api_key: key,
        created_at: Utc::now().timestamp(),
        last_used_at: None,
        note: note.map(|s| s.to_string()),
    })?;
    println!(
        "Stored profile '{}' to {}.",
        resolved_name,
        crate::paths::credentials_db_path()?.display()
    );
    Ok(())
}

/// SPEC §5.2: take the API key's last 6 characters; if that name is
/// already taken append `(1)`, `(2)`, ... until free. Bails if the key
/// is shorter than 6 chars or the tail contains characters
/// `validate_profile_name` rejects (non-ASCII / disallowed punctuation).
fn auto_pick_name(db: &store::CredStore, key: &str) -> Result<String> {
    if key.chars().count() < 6 {
        user_bail!(
            "API key is shorter than 6 characters; please provide an explicit profile \
             name: `a2a auth add <name> [--from-stdin]`"
        );
    }
    let tail: String = key
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    // Validate the base name once up-front. If it fails (e.g. the key
    // tail contains non-ASCII or disallowed chars — Cursor keys never
    // do, but a non-Cursor `--from-stdin` key might), bail immediately
    // instead of looping 1000 times trying `tail(1)`, `tail(2)`, ...
    // that would all fail the same validation.
    if let Err(e) = validate_profile_name(&tail) {
        user_bail!(
            "cannot auto-derive profile name from key tail '{tail}' ({e}); please \
             provide an explicit name: `a2a auth add <name> [--from-stdin]`"
        );
    }
    if !db.profile_exists(&tail)? {
        return Ok(tail);
    }
    for n in 1..1000 {
        let candidate = format!("{tail}({n})");
        if !db.profile_exists(&candidate)? {
            return Ok(candidate);
        }
    }
    user_bail!(
        "tried 1000 candidate names with suffix from key tail '{tail}'; please specify \
         a name explicitly"
    );
}

pub fn cmd_list() -> Result<()> {
    let db = store::open()?;
    let profiles = db.list_profiles()?;
    let default_profile = db.get_default_profile()?;
    println!(
        "Profiles in {}:",
        crate::paths::credentials_db_path()?.display()
    );
    if profiles.is_empty() {
        println!("  (no profiles configured; run `a2a auth add` to add one)");
        return Ok(());
    }
    for p in &profiles {
        let mark = if Some(&p.name) == default_profile.as_ref() {
            "*"
        } else {
            " "
        };
        let last_used = p
            .last_used_at
            .and_then(|t| chrono::DateTime::<Utc>::from_timestamp(t, 0))
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "never".into());
        println!(
            "{} {:<14}  last_used={:<16}{}",
            mark,
            p.name,
            last_used,
            p.note
                .as_deref()
                .map(|n| format!("  -- {n}"))
                .unwrap_or_default()
        );
    }
    println!();
    println!("(* = default profile; set with `a2a auth use <name>`)");
    Ok(())
}

pub fn cmd_use(name: &str) -> Result<()> {
    let mut db = store::open()?;
    if !db.profile_exists(name)? {
        user_bail!("profile not found: {name}");
    }
    db.set_default_profile(name)?;
    println!("Default profile set to '{name}'.");
    Ok(())
}

pub fn cmd_show(name: &str) -> Result<()> {
    let mut db = store::open()?;
    let key = read_api_key(&mut db, name)?;
    println!("{} -> {}", name, mask_key(&key));
    Ok(())
}

pub fn cmd_remove(name: &str, yes: bool) -> Result<()> {
    let mut db = store::open()?;
    if !db.profile_exists(name)? {
        user_bail!("profile not found: {name}");
    }
    // No project-level config to scan for dangling references —
    // profile bindings are CLI-time only (`a2a ask --profiles a,b,c`).
    // Removing a profile can only break a future explicit `--profiles`
    // argument, which the fallback runner reports as `not_found`.

    if !yes {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!("Remove profile '{name}'? This cannot be undone."))
            .default(false)
            .interact()
            .context("confirmation prompt failed")?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }
    db.delete_profile(name)?;
    if db.get_default_profile()?.as_deref() == Some(name) {
        db.clear_default_profile()?;
    }
    println!("Removed profile '{name}'.");
    Ok(())
}

pub fn cmd_update(name: &str, from_stdin: bool) -> Result<()> {
    let mut db = store::open()?;
    if !db.profile_exists(name)? {
        user_bail!("profile not found: {name}");
    }
    let new_key = if from_stdin {
        read_first_stdin_line().context("read API key from stdin")?
    } else {
        prompt_api_key()?
    };
    if new_key.is_empty() {
        user_bail!("empty API key");
    }
    if !looks_like_cursor_key(&new_key) {
        println!(
            "warning: key does not match expected Cursor token prefixes (key_ / crsr_); accepting anyway"
        );
    }
    db.update_profile_key(name, &new_key)?;
    println!("Updated profile '{name}'.");
    Ok(())
}

/// SPEC §6.3: when KeyDead is detected during a fallback chain run,
/// the orchestrator calls this to remove the broken profile so the
/// chain advances. Returns `true` if the credentials store became
/// **empty** as a result of the delete; the caller (fallback runner)
/// uses that signal to abort the whole `a2a ask` rather than letting
/// every remaining alias also waste a cursor-agent call against a
/// store that has no profiles left.
pub fn delete_profile_on_key_dead(db: &mut store::CredStore, name: &str) -> Result<bool> {
    db.delete_profile(name)?;
    if db.get_default_profile()?.as_deref() == Some(name) {
        db.clear_default_profile()?;
    }
    db.is_empty()
}

// ---------- internal helpers ----------

fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() {
        user_bail!("profile name cannot be empty");
    }
    // SPEC §5.1: ASCII alphanumeric / `-` / `_` plus `(` `)` for the
    // auto-suffix `(1)` `(2)` form.
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '(' || c == ')')
    {
        user_bail!("profile name must be alphanumeric / '-' / '_' / '(' / ')': {name}");
    }
    Ok(())
}

fn prompt_api_key() -> Result<String> {
    dialoguer::Password::new()
        .with_prompt("Enter Cursor API key (input hidden)")
        .with_confirmation("Confirm", "Keys did not match")
        .interact()
        .context("password prompt failed")
}

/// Read first non-empty line from stdin. Strips UTF-8 BOM (PowerShell
/// `"text" | command` injects one on Windows) and whitespace.
fn read_first_stdin_line() -> Result<String> {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    loop {
        let mut line = String::new();
        let n = handle.read_line(&mut line).context("read stdin")?;
        if n == 0 {
            user_bail!("stdin closed before any non-empty line was read");
        }
        let cleaned: String = line.trim_start_matches('\u{feff}').trim().to_string();
        if cleaned.is_empty() {
            continue;
        }
        return Ok(cleaned);
    }
}

/// Best-effort sanity check for a Cursor token. Returns `false` for
/// anything not matching known prefixes; callers warn but accept.
fn looks_like_cursor_key(s: &str) -> bool {
    if s.len() < 16 {
        return false;
    }
    ["key_", "crsr_", "sk_"].iter().any(|p| s.starts_with(p))
}

fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "****".into();
    }
    let head: String = chars.iter().take(4).copied().collect();
    let tail_rev: Vec<char> = chars.iter().rev().take(4).copied().collect();
    let tail: String = tail_rev.iter().rev().copied().collect();
    format!("{head}****{tail}")
}

/// Read a profile's API key. No decryption needed; SPEC §2.1.
pub fn read_api_key(db: &mut store::CredStore, name: &str) -> Result<String> {
    let p = db.get_profile(name)?.ok_or_else(|| {
        anyhow::Error::new(crate::UserError(format!("profile not found: {name}")))
    })?;
    Ok(p.api_key)
}
