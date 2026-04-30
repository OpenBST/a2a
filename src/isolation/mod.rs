// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! Workspace isolation: copy declared context files into an OS tempdir
//! that cursor-agent gets via `--workspace`. SPEC §7.
//!
//! Only one mode: `readonly_mirror`. No scratch / git worktree (SPEC §2.2).
//! No NTFS ADS / Windows drive-relative checks (SPEC §7.2 — user machine
//! is trusted; canonicalize + starts_with(project_root) is enough).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Hard-coded skip set for directory walks. These are never useful as
/// model context and would either leak prior consultations or balloon
/// the mirror.
const SKIP_DIRS: [&str; 5] = ["target", "node_modules", ".git", ".a2a", "consultations"];

pub struct IsolatedWorkspace {
    root: PathBuf,
}

impl IsolatedWorkspace {
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Verify the workspace root still exists. If it doesn't, another
    /// process has deleted it mid-run (most commonly a concurrent
    /// `a2a clean --yes`), so the current run cannot meaningfully
    /// continue. SPEC §3 / §7: callers (the fallback runner) check
    /// this at chokepoints — after cursor-agent returns, before
    /// writing the answer file — and abort the model alias with a
    /// clear error pointing at the likely cause.
    pub fn assert_alive(&self) -> Result<()> {
        if !self.root.exists() {
            anyhow::bail!(
                "isolation workspace {} no longer exists — another a2a process \
                 likely deleted it (e.g. `a2a clean --yes` while this run was in \
                 flight). Aborting this model alias.",
                self.root.display()
            );
        }
        Ok(())
    }
}

impl Drop for IsolatedWorkspace {
    fn drop(&mut self) {
        // Plain remove. No git worktree registration to clean up
        // (scratch mode is gone), no detached cleanup thread (the
        // remove is fast — no cargo `target/` either since it's in
        // SKIP_DIRS).
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Create a readonly_mirror tempdir and copy the declared context
/// files into it. Returns the workspace; cleanup happens on Drop.
///
/// SPEC §11: the project-level `always_include` concept
/// is gone; the entire context surface is the prompt frontmatter's
/// `context_files` list, plus whatever `a2a-multi-ai-consult` /
/// `a2a-operator` skill conventions tell the agent to add.
///
/// Path validation: each entry must be project-relative; after
/// canonicalisation the result must remain under `project_root`. Entries
/// that fail are skipped with a tracing::warn!.
pub fn create_readonly_mirror(
    project_root: &Path,
    context_files: &[String],
) -> Result<IsolatedWorkspace> {
    let tmp_root = std::env::temp_dir().join(format!("a2a-{}", Uuid::new_v4().simple()));
    create_dir_private(&tmp_root)?;

    let canonical_project = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());

    for entry in context_files {
        let rel = Path::new(entry);
        if rel.is_absolute() {
            tracing::warn!("context entry rejected (absolute path): {entry}");
            continue;
        }
        // Reject `..` traversal at the syntactic level so we don't
        // even hit the filesystem with an obviously bad path.
        if rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            tracing::warn!("context entry rejected (parent traversal): {entry}");
            continue;
        }
        let src = project_root.join(rel);
        let canonical_src = match src.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!("context entry missing, skipping: {}", src.display());
                continue;
            }
        };
        // Symlink jailbreak guard: after canonicalize the result must
        // still live under project_root.
        if !canonical_src.starts_with(&canonical_project) {
            tracing::warn!(
                "context entry rejected (escapes project root): {}",
                canonical_src.display()
            );
            continue;
        }

        let dst = tmp_root.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if canonical_src.is_file() {
            std::fs::copy(&canonical_src, &dst).with_context(|| {
                format!("copy {} -> {}", canonical_src.display(), dst.display())
            })?;
        } else if canonical_src.is_dir() {
            copy_dir_filtered(&canonical_src, &dst, &canonical_project)?;
        }
    }

    Ok(IsolatedWorkspace { root: tmp_root })
}

/// Create a directory with restrictive permissions (0700) without a
/// TOCTOU window where it briefly exists as world-readable. Unix uses
/// `DirBuilder::mode`; Windows inherits the parent's user-private ACL
/// (`%TEMP%` is already user-private).
fn create_dir_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .with_context(|| format!("create private dir {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path).with_context(|| format!("create dir {}", path.display()))?;
    }
    Ok(())
}

/// Recursive directory copy with hard-coded skip list. Skips
/// SKIP_DIRS at any depth. Symlinked files are followed (so a context
/// declaration via symlink works) but their canonical target must
/// still live under `canonical_project`.
fn copy_dir_filtered(src: &Path, dst: &Path, canonical_project: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(src).into_iter().filter_entry(|e| {
        e.depth() == 0
            || !e.file_type().is_dir()
            || !SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref())
    }) {
        let entry = entry?;
        let rel = match entry.path().strip_prefix(src) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dst.join(&rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() || entry.file_type().is_symlink() {
            // Symlink: canonicalize target, verify (a) it resolves,
            // (b) it stays under the project root, (c) it points to
            // a regular file (not a directory — `std::fs::copy` only
            // handles files; symlink-to-dir would otherwise abort
            // the whole mirror with `is a directory`).
            if entry.file_type().is_symlink() {
                let canon = match std::fs::canonicalize(entry.path()) {
                    Ok(c) => c,
                    Err(_) => {
                        tracing::warn!(
                            "skipping broken / unresolvable symlink: {}",
                            entry.path().display()
                        );
                        continue;
                    }
                };
                if !canon.starts_with(canonical_project) {
                    tracing::warn!(
                        "skipping symlink that escapes project root: {} -> {}",
                        entry.path().display(),
                        canon.display()
                    );
                    continue;
                }
                if canon.is_dir() {
                    // Symlink-to-dir inside an allowed declared
                    // directory. `std::fs::copy` cannot copy a
                    // directory, and recursing would risk infinite
                    // loops (a -> b -> a). Skip with a warning.
                    tracing::warn!(
                        "skipping symlink-to-directory inside declared dir: {} -> {} \
                         (declare the target directory explicitly in `context_files` if needed)",
                        entry.path().display(),
                        canon.display()
                    );
                    continue;
                }
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
