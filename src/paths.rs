// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! Cross-platform path resolution and template installation.

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use std::path::{Path, PathBuf};

/// Find the a2a project root by walking up from `start` looking for
/// a `.a2a/` directory. Returns `Some(<dir>)` when a project marker
/// is found, `None` when the walk reached the filesystem root with
/// no match. SPEC §11: `.a2a/` is the only marker (no `.git/`
/// fallback — that caused cross-project pollution when a third-party
/// repo sat inside an outer git repo).
///
/// **The user's home directory is explicitly skipped** even if its
/// `.a2a/` exists — that path is the credentials store
/// (`~/.a2a/credentials.db`), not a project root. Without this
/// carve-out, running `a2a` anywhere under `%USERPROFILE%` would
/// stop the walk at the home dir and mis-report it as "initialised".
///
/// Note for callers: do **not** convert `Some(root)` into
/// `root.join(".a2a").is_dir()` — that re-introduces the home-dir
/// false positive (because the credentials store *is* `~/.a2a/`).
/// `is_some()` is the answer to "is this an initialised project?".
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let home = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());
    let mut cur = start.to_path_buf();
    loop {
        if cur.join(".a2a").is_dir() && Some(&cur) != home.as_ref() {
            return Some(cur);
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => return None,
        }
    }
}

/// Resolve the a2a project root from the current working directory,
/// or fall back to cwd itself when no project marker is found above.
/// Used by commands that always need a working directory but treat
/// "no project initialised" as a soft state (consultations / clean /
/// ask: the orchestrator validates inside).
///
/// Callers that genuinely need to distinguish "found a project" from
/// "no project here" should call [`find_project_root`] directly.
pub fn project_root_from_cwd() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("failed to detect cwd")?;
    Ok(find_project_root(&cwd).unwrap_or(cwd))
}

/// Path to the user-level a2a data directory: `~/.a2a/` on Unix-like,
/// `%USERPROFILE%\.a2a\` on Windows.
pub fn user_data_dir() -> Result<PathBuf> {
    let home = directories::UserDirs::new()
        .ok_or_else(|| anyhow!("could not resolve user home directory"))?
        .home_dir()
        .to_path_buf();
    Ok(home.join(".a2a"))
}

pub fn ensure_user_data_dir() -> Result<PathBuf> {
    let dir = user_data_dir()?;
    // On Unix create the directory directly with mode 0700 to close the
    // TOCTOU window between mkdir(0755) and chmod(0700). DirBuilder with
    // mode set means the directory is never world-readable, even for an
    // instant; no other local user can grab a directory descriptor and
    // read credentials.db moments later.
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        if !dir.exists() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        } else {
            // Existing dir — defensively tighten in case it was created
            // earlier with a wider mode.
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
    }
    Ok(dir)
}

/// SQLite credential database path: `~/.a2a/credentials.db`.
///
/// On first creation we tighten the file permissions to be readable only
/// by the current user. On Unix this is `0600`. On Windows the file
/// inherits ACLs from `~/.a2a` (which itself is in `%USERPROFILE%\` and
/// already user-private under default Windows ACL inheritance), so we
/// rely on that rather than calling `icacls`.
pub fn credentials_db_path() -> Result<PathBuf> {
    let path = ensure_user_data_dir()?.join("credentials.db");
    #[cfg(unix)]
    if path.is_file() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(path)
}

/// Per-user state file (default profile selection, etc.).
pub fn user_state_path() -> Result<PathBuf> {
    Ok(ensure_user_data_dir()?.join("state.toml"))
}

/// Project marker directory: `<project>/.a2a/`. SPEC §11: this
/// directory exists purely as the marker that `find_project_root`
/// looks for, plus as a stable home for the readonly_mirror's
/// `_prompt.md` indirect-prompt scratch file. There is no
/// `.a2a/config.toml`; all configuration lives in
/// `~/.a2a/credentials.db` or hardcoded constants.
pub fn project_config_dir(project_root: &Path) -> PathBuf {
    project_root.join(".a2a")
}

pub fn project_consultations_dir(project_root: &Path) -> PathBuf {
    project_root.join("consultations")
}

/// Install bundled Cursor templates (compiled into the binary via
/// `crate::embedded`) into the target project. Each template is
/// written to TWO locations:
///   - `<project>/.a2a/template/<stage_rel>`: staged raw copy that
///     mirrors the source-tree layout. Acts as a per-project audit
///     log so a user can inspect exactly what a2a put on disk
///     (with `{{A2A_VERSION}}` already substituted).
///   - `<project>/<dst_rel>`: the live copy Cursor actually loads
///     (`.cursor/skills/...` / `.cursor/rules/...` / etc.).
///
/// Also creates the `.a2a/` project marker dir, `consultations/`,
/// and the `consultations/.gitignore` hint file.
///
/// SPEC §11: there is no `.a2a/config.toml`; all configuration
/// lives in `~/.a2a/credentials.db` (user-global) or hardcoded
/// constants (`crate::defaults`).
pub fn install_templates_into_project(project_root: &Path, force: bool) -> Result<()> {
    let dst_a2a_dir = project_config_dir(project_root);
    let dst_consult = project_consultations_dir(project_root);
    let stage_root = dst_a2a_dir.join("template");
    std::fs::create_dir_all(&dst_a2a_dir)?;
    std::fs::create_dir_all(&dst_consult)?;
    std::fs::create_dir_all(&stage_root)?;

    for asset in crate::embedded::TEMPLATE_ASSETS {
        // Defensive: even though `TEMPLATE_ASSETS` is a `const` slice
        // (compile-time-known), funnel both rel paths through the
        // project-relative validator so a future contributor adding
        // e.g. `dst_rel: "../escape.txt"` cannot silently traverse
        // out of the project via `Path::join`'s `..` semantics.
        crate::runner::validate_project_relative(
            asset.stage_rel,
            "embedded TemplateAsset.stage_rel",
        )?;
        crate::runner::validate_project_relative(asset.dst_rel, "embedded TemplateAsset.dst_rel")?;
        let body = asset.materialised();
        let stage = stage_root.join(asset.stage_rel);
        write_template_to(&stage, &body, true)?;
        let dst = project_root.join(asset.dst_rel);
        write_template_to(&dst, &body, force)?;
    }

    let dst_gitignore = dst_consult.join(".gitignore");
    if !dst_gitignore.exists() {
        // The whole consultations/ tree is already a gitignore target
        // by convention (per-run dirs contain answers + meta). This
        // file just makes the convention explicit for `git add` UX.
        std::fs::write(
            &dst_gitignore,
            "# auto-generated by a2a init\n# consult dirs (timestamped) are user-private; ignore everything\n*\n",
        )?;
        println!("created {}", dst_gitignore.display());
    }

    println!();
    println!("a2a templates installed.");
    print_post_init_next_step();
    Ok(())
}

/// Imperative next-step block printed at the end of `a2a init`.
/// Worded for AI agents (English; structured) so Cursor's main agent
/// can echo it to the user verbatim. Restart-Cursor + type-`a2a_guide`
/// is the only path that gets the freshly-installed setup-guide
/// skill loaded into Cursor's runtime — there is no in-process
/// shortcut.
fn print_post_init_next_step() {
    println!();
    println!("================================================================");
    println!("  NEXT STEP — restart required for skill activation");
    println!("================================================================");
    println!();
    println!("If you are an AI agent: tell the user to do these 4 things, in order:");
    println!();
    println!("  1. Close Cursor completely (every window, including the welcome screen).");
    println!("  2. Reopen Cursor on this project.");
    println!("  3. Open a NEW chat in this project.");
    println!("  4. Type the single word, exactly: a2a_guide");
    println!();
    println!("That triggers the freshly-installed `a2a-setup-guide` skill, which");
    println!("walks the user through registering a Cursor API key and registering");
    println!("at least one model alias. After that, `a2a ask <topic> --prompt-file");
    println!("<path>` is ready to use.");
    println!();
    println!("Restart is REQUIRED — Cursor only loads new skills under .cursor/skills/");
    println!("on startup; without a restart the skill won't be visible to the agent");
    println!("in the current session.");
}

/// Write `body` to `dst`. Behaviour:
/// - existing identical content prints `ok     ...` (no write).
/// - existing different content prints `kept   ... (locally modified;
///   re-run with --force to overwrite)` and skips the write.
/// - missing path, or `force = true`, writes the file.
fn write_template_to(dst: &Path, body: &str, force: bool) -> Result<()> {
    if dst.exists() && !force {
        let existing = std::fs::read(dst).with_context(|| format!("read {}", dst.display()))?;
        if existing == body.as_bytes() {
            println!("ok     {} (matches template)", dst.display());
        } else {
            println!(
                "kept   {} (locally modified; re-run with --force to overwrite)",
                dst.display()
            );
        }
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dst, body).with_context(|| format!("write {}", dst.display()))?;
    println!("wrote  {}", dst.display());
    Ok(())
}

/// Convenience helper for retrieving the user's `ProjectDirs` if needed
/// (currently unused; reserved for future cache directory).
#[allow(dead_code)]
pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("dev", "a2a", "a2a")
}
