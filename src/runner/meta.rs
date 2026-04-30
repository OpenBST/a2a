// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! Metadata persistence: write meta.toml describing each consultation run.
//!
//! SPEC §13 / §14: every successful or failed model alias appends one
//! `[[models]]` row. When `--log-budget` is set, the row also carries
//! a `[models.budget]` sub-table with char counts (token-proxy).
//!
//! There is **no separate `budget.toml`** — the audit trail and the
//! cost-proxy log live in the same file (one less file to grep, one
//! less file-lock to manage).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub topic: String,
    pub created_at: DateTime<Utc>,
    pub a2a_version: String,
    pub models: Vec<ModelMeta>,
    pub command_line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMeta {
    pub alias: String,
    pub cursor_model: String,
    pub mode: String,
    pub profile_used: String,
    pub fallback_chain: Vec<String>,
    pub fallback_attempts: Vec<FallbackAttempt>,
    pub success: bool,
    pub elapsed_ms: u64,
    pub answer_path: PathBuf,
    /// SPEC §14.4: every cursor-agent invocation's session_id, in
    /// invocation order. A single alias usually has one entry
    /// (transient retry reuses the same session_id, cross-account
    /// resume on KeyDead may create a new one). Empty when no
    /// init event ever arrived (early hard failure / dry_run).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_ids: Vec<String>,
    /// SPEC §14.4: convenience field — last `session_ids` entry, the
    /// id you'd pass to `cursor-agent --resume <id>` to manually
    /// continue. `None` if `session_ids` is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_session_id: Option<String>,
    /// SPEC §14: char-count audit (token proxy). Only populated when
    /// the user opted in via `a2a ask --log-budget`. Absent otherwise
    /// — `#[serde(skip_serializing_if = "Option::is_none")]` keeps the
    /// `[models.budget]` table out of meta.toml when not requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetInfo>,
}

/// SPEC §14: per-model char-count breakdown. Topic / model / profile /
/// timestamp / elapsed_ms / success are already at the `Meta` /
/// `ModelMeta` level; this struct only holds the four char counts
/// that aren't otherwise represented.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetInfo {
    pub prompt_chars: usize,
    pub context_chars: usize,
    pub always_chars: usize,
    pub answer_chars: usize,
}

/// SPEC §6: one row per profile attempted. `error_class` is the
/// human-readable lowercase form of `ErrorClass` (`keydead` /
/// `modelunavailable` / `transient` / `unknown`) when failure;
/// None on success. `error_excerpt` holds the last 8 lines of stderr.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackAttempt {
    pub profile: String,
    pub success: bool,
    pub error_class: Option<String>,
    pub error_excerpt: Option<String>,
    pub elapsed_ms: u64,
    /// SPEC §14.4: cursor-agent's session_id (chatId) for this
    /// specific attempt. `None` when stream-json never reached the
    /// init event (e.g. spawn error, key rejected before HTTP, or
    /// `--dry-run`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl Meta {
    pub fn write(&self, dir: &Path) -> Result<()> {
        let path = dir.join("meta.toml");
        let s = toml::to_string_pretty(self).context("serialize meta.toml")?;
        crate::util::file_lock::atomic_write(&path, &s)?;
        Ok(())
    }
}

/// Append a model record to the on-disk meta.toml (creating it if needed).
///
/// Safe for concurrent calls from multiple parallel model tasks: a
/// `<consult_dir>/.meta.toml.lock` advisory lock serialises the
/// read-modify-write cycle, and `atomic_write` uses tmp+rename so a
/// crash mid-write leaves the previous file intact.
pub async fn append_model_meta(dir: &Path, topic: &str, m: ModelMeta) -> Result<()> {
    let path = dir.join("meta.toml");
    let _lock = crate::util::file_lock::FileLock::acquire(&path, Duration::from_secs(30)).await?;

    let mut existing: Meta = if path.exists() {
        let raw =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        match toml::from_str(&raw) {
            Ok(m) => m,
            Err(e) => {
                // Corrupt meta.toml: rename to `meta.toml.corrupt-<ts>`
                // so any prior models' entries aren't silently wiped
                // by the fresh write.
                let stamp = chrono::Utc::now().timestamp();
                let corrupt_path = path.with_extension(format!("toml.corrupt-{stamp}"));
                if let Err(rename_err) = std::fs::rename(&path, &corrupt_path) {
                    tracing::warn!(
                        "meta.toml parse failed ({e}); could not rename to {}: {rename_err:#} — \
                         starting fresh and dropping prior content (DATA LOSS)",
                        corrupt_path.display()
                    );
                } else {
                    tracing::warn!(
                        "meta.toml parse failed ({e}); preserved corrupt file as {} and \
                         starting fresh — sibling models' entries from the prior file are \
                         in that backup, not in the new meta.toml",
                        corrupt_path.display()
                    );
                }
                new_meta(topic)
            }
        }
    } else {
        new_meta(topic)
    };
    existing.models.push(m);
    let s = toml::to_string_pretty(&existing).context("serialize meta.toml")?;
    crate::util::file_lock::atomic_write(&path, &s)?;
    Ok(())
}

fn new_meta(topic: &str) -> Meta {
    Meta {
        topic: topic.to_string(),
        created_at: Utc::now(),
        a2a_version: crate::A2A_VERSION.to_string(),
        models: Vec::new(),
        command_line: std::env::args().collect::<Vec<_>>().join(" "),
    }
}
