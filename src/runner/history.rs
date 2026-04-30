// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! `a2a list` and `a2a clean` implementation.
//!
//! SPEC §3 / §15. No git worktree pruning (scratch mode is gone).

use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;

/// Resolve the consultations directory for `project_root`. SPEC §11
/// SPEC §11: `output_root` is the hardcoded constant
/// `crate::defaults::OUTPUT_ROOT` (no project-level override). The
/// symlink-escape canonicalize check stays — a junction at
/// `consultations/` (or any ancestor) pointing outside the project
/// must not let `list` / `clean` operate outside.
fn consultations_dir(project_root: &Path) -> Result<std::path::PathBuf> {
    let value = crate::defaults::OUTPUT_ROOT;
    crate::runner::validate_project_relative(value, "consultations output root")?;
    let canon_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let resolved = canon_root.join(value);
    let canon_anchor = crate::runner::deepest_canonical_ancestor(&resolved)?;
    if !canon_anchor.starts_with(&canon_root) {
        anyhow::bail!(
            "consultations output root '{value}' resolves under {} which escapes the \
             project root {} (an existing ancestor is a symlink/junction). \
             Refusing to operate outside the project.",
            canon_anchor.display(),
            canon_root.display()
        );
    }
    Ok(resolved)
}

pub fn list(project_root: &Path) -> Result<()> {
    let dir = consultations_dir(project_root)?;
    if !dir.is_dir() {
        println!("(no consultations directory in {})", project_root.display());
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|f| f.is_dir()).unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    if entries.is_empty() {
        println!("(no past consultations)");
        return Ok(());
    }
    println!("Consultations under {}:", dir.display());
    for e in entries {
        println!("  {}", e.file_name().to_string_lossy());
    }
    Ok(())
}

pub fn clean(project_root: &Path, older_than: Option<&str>, yes: bool) -> Result<()> {
    let dir = consultations_dir(project_root)?;
    let now = Utc::now().timestamp();

    // SPEC §3: `--older-than` parses as a relative duration only.
    // Absolute dates / RFC-3339 are rejected — the prior reuse of
    // `parse_disable_until` (which accepted them) created data-loss
    // scenarios. Format: `<N>(s|m|h|d|w)`.
    let cutoff_mtime: Option<i64> = if let Some(spec) = older_than {
        let duration_secs = parse_older_than_duration(spec)
            .with_context(|| format!("invalid --older-than spec: {spec}"))?;
        if duration_secs <= 0 {
            return Err(anyhow::anyhow!(
                "--older-than '{spec}' resolves to a past or zero duration; \
                 use e.g. '30d', '7d', '1h'"
            ));
        }
        Some(now - duration_secs)
    } else {
        None
    };

    let mut victims: Vec<std::path::PathBuf> = Vec::new();

    // Pass 1: project-level consultations directory.
    // Only delete subdirs that have a meta.toml (verifiably an a2a
    // consultation, not arbitrary user content).
    if dir.is_dir() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if !ft.is_dir() {
                continue;
            }
            if !entry.path().join("meta.toml").is_file() {
                tracing::warn!(
                    "skipping {} (no meta.toml — not a consultation dir)",
                    entry.path().display()
                );
                continue;
            }
            if let Some(threshold) = cutoff_mtime {
                let mtime = match entry.metadata().and_then(|m| m.modified()).map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64
                }) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if mtime >= threshold {
                    continue;
                }
            }
            victims.push(entry.path());
        }
    }

    // Pass 2: OS tempdir leftovers from crashed runs (`a2a-<uuid>`).
    let temp_root = std::env::temp_dir();
    if temp_root.is_dir() {
        for entry in std::fs::read_dir(&temp_root)? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !is_a2a_temp_name(&name_str) {
                continue;
            }
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if !ft.is_dir() {
                continue;
            }
            if let Some(threshold) = cutoff_mtime {
                let mtime = match entry.metadata().and_then(|m| m.modified()).map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64
                }) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if mtime >= threshold {
                    continue;
                }
            }
            victims.push(entry.path());
        }
    }

    if victims.is_empty() {
        println!("nothing matches.");
        return Ok(());
    }
    // Count how many victims came from Pass 2 (OS tempdir) so the
    // confirmation prompt can warn about the active-mirror collision
    // risk specifically. Pass 1 victims live under `consultations/`
    // (project-local, never an active workspace).
    let temp_root = std::env::temp_dir();
    let active_mirror_candidates: usize =
        victims.iter().filter(|v| v.starts_with(&temp_root)).count();
    if !yes {
        println!("would remove:");
        for v in &victims {
            println!("  {}", v.display());
        }
        println!();
        if active_mirror_candidates > 0 {
            println!(
                "WARNING: {active_mirror_candidates} of the entries above are OS tempdir \
                 directories (`a2a-<uuid>`). These are workspace mirrors of crashed runs \
                 — but if any other `a2a ask` is currently running, its workspace tempdir \
                 may be among them and removing it will cause that run to abort with a \
                 'workspace deleted mid-run' error."
            );
            println!();
        }
        let prompt = if active_mirror_candidates > 0 {
            format!(
                "Confirm no other `a2a` process is currently running, then remove {} entry(ies)?",
                victims.len()
            )
        } else {
            format!("Remove {} entry(ies)?", victims.len())
        };
        let confirm = dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()
            .context("confirmation prompt")?;
        if !confirm {
            println!("aborted.");
            return Ok(());
        }
    }
    for v in victims {
        match std::fs::remove_dir_all(&v) {
            Ok(_) => println!("removed {}", v.display()),
            Err(e) => {
                tracing::warn!("could not remove {}: {e:#}", v.display());
            }
        }
    }
    Ok(())
}

/// Parse a relative duration like `30d`, `7d`, `1h` into seconds.
/// Suffix s/m/h/d/w (lowercase). Rejects absolute dates / RFC-3339 /
/// `next_month_first_day` / negative numbers / mixed-case suffixes.
fn parse_older_than_duration(spec: &str) -> anyhow::Result<i64> {
    let t = spec.trim();
    if t.is_empty() {
        anyhow::bail!("empty --older-than spec");
    }
    // Use char-based suffix-strip rather than byte indexing so a
    // multi-byte input like `30天` doesn't panic at `t[..t.len()-1]`
    // (which would slice mid-codepoint).
    let (num_part, last) = match (
        t.strip_suffix('s'),
        t.strip_suffix('m'),
        t.strip_suffix('h'),
        t.strip_suffix('d'),
        t.strip_suffix('w'),
    ) {
        (Some(rest), _, _, _, _) => (rest, 's'),
        (_, Some(rest), _, _, _) => (rest, 'm'),
        (_, _, Some(rest), _, _) => (rest, 'h'),
        (_, _, _, Some(rest), _) => (rest, 'd'),
        (_, _, _, _, Some(rest)) => (rest, 'w'),
        _ => anyhow::bail!(
            "expected a relative duration like '30d', '7d', '1h' (suffix s|m|h|d|w); \
             absolute dates / RFC-3339 are not supported here"
        ),
    };
    let n: i64 = num_part
        .parse()
        .with_context(|| format!("could not parse '{num_part}' as a non-negative integer"))?;
    if n < 0 {
        anyhow::bail!("negative duration not allowed: {spec}");
    }
    let multiplier: i64 = match last {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 60 * 60 * 24,
        'w' => 60 * 60 * 24 * 7,
        _ => unreachable!(),
    };
    Ok(n * multiplier)
}

/// SPEC §15 housekeeping: best-effort delete of consultation dirs
/// older than `cutoff_unix_secs`. Only removes dirs that contain a
/// `meta.toml` (so a user dropping random files into `consultations/`
/// won't lose them). Errors are `tracing::warn!`'d, never bubbled —
/// this is housekeeping, not a load-bearing operation.
pub fn housekeep_consults_in(consult_root: &Path, cutoff_unix_secs: i64) {
    let dir = match std::fs::read_dir(consult_root) {
        Ok(d) => d,
        Err(_) => return,
    };
    for entry in dir.filter_map(|e| e.ok()) {
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !ft.is_dir() {
            continue;
        }
        if !entry.path().join("meta.toml").is_file() {
            continue;
        }
        let mtime = match entry.metadata().and_then(|m| m.modified()).map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        }) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if mtime >= cutoff_unix_secs {
            continue;
        }
        if let Err(e) = std::fs::remove_dir_all(entry.path()) {
            tracing::warn!(
                "housekeep: could not remove {}: {e:#}",
                entry.path().display()
            );
        }
    }
}

/// Spawn a detached OS thread that prunes stale consult dirs. Called
/// from `ask_orchestrator` at startup so each `a2a ask` pays a tiny
/// housekeeping cost.
///
/// Uses `std::thread::spawn` (not `tokio::spawn`) so the thread is
/// independent of the tokio runtime — when `ask` finishes and the
/// runtime drops, the thread keeps running until the process exits
/// or the thread completes (typically a few hundred ms for a normal
/// project). Worst case: the process exits before the thread
/// finishes and the next `a2a ask` picks up the leftovers.
pub fn spawn_housekeep_old_consults(consult_root: std::path::PathBuf, retain_days: u64) {
    std::thread::spawn(move || {
        let cutoff = Utc::now().timestamp() - (retain_days as i64) * 86400;
        housekeep_consults_in(&consult_root, cutoff);
    });
}

/// Match only directory names a2a itself produces: `a2a-<32-hex>`.
/// Strict matcher prevents `a2a clean` from accidentally deleting
/// unrelated user dirs (`a2a-cargo`, `a2a-tools`, etc.).
fn is_a2a_temp_name(name: &str) -> bool {
    let rest = match name.strip_prefix("a2a-") {
        Some(r) => r,
        None => return false,
    };
    rest.len() == 32 && rest.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod temp_name_tests {
    use super::is_a2a_temp_name;

    #[test]
    fn matches_uuid_simple_forms() {
        assert!(is_a2a_temp_name("a2a-0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn rejects_user_dirs() {
        assert!(!is_a2a_temp_name("a2a-cargo"));
        assert!(!is_a2a_temp_name("a2a-build-cache"));
        assert!(!is_a2a_temp_name("a2a-tools-2024"));
        assert!(!is_a2a_temp_name("a2a-"));
        assert!(!is_a2a_temp_name("a2a"));
    }

    #[test]
    fn rejects_uuid_with_dashes() {
        assert!(!is_a2a_temp_name(
            "a2a-01234567-89ab-cdef-0123-456789abcdef"
        ));
    }

    #[test]
    fn rejects_scratch_prefix() {
        // SPEC: scratch mode removed; only `a2a-<uuid>` is valid.
        assert!(!is_a2a_temp_name(
            "a2a-scratch-0123456789abcdef0123456789abcdef"
        ));
    }
}

#[cfg(test)]
mod older_than_tests {
    use super::parse_older_than_duration;

    #[test]
    fn accepts_relative_durations() {
        assert_eq!(parse_older_than_duration("30s").unwrap(), 30);
        assert_eq!(parse_older_than_duration("5m").unwrap(), 5 * 60);
        assert_eq!(parse_older_than_duration("2h").unwrap(), 2 * 60 * 60);
        assert_eq!(parse_older_than_duration("7d").unwrap(), 7 * 86400);
        assert_eq!(parse_older_than_duration("1w").unwrap(), 7 * 86400);
    }

    #[test]
    fn rejects_uppercase_suffix() {
        assert!(parse_older_than_duration("30D").is_err());
    }

    #[test]
    fn rejects_absolute_date() {
        assert!(parse_older_than_duration("2030-01-01T00:00:00Z").is_err());
        assert!(parse_older_than_duration("next_month_first_day").is_err());
    }

    #[test]
    fn rejects_multibyte_without_panic() {
        // Regression: multibyte char as suffix used to panic via
        // byte-index slicing past a UTF-8 boundary.
        assert!(parse_older_than_duration("30天").is_err());
        assert!(parse_older_than_duration("天").is_err());
        assert!(parse_older_than_duration("日").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_older_than_duration("").is_err());
        assert!(parse_older_than_duration("   ").is_err());
    }

    #[test]
    fn rejects_negative() {
        assert!(parse_older_than_duration("-30d").is_err());
    }

    #[test]
    fn rejects_no_suffix() {
        assert!(parse_older_than_duration("30").is_err());
    }
}
