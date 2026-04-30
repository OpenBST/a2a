// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! Orchestration: ask N models in parallel via cursor-agent subprocesses.
//!
//! SPEC §3.1 / §9 / §11 / §12. No scratch isolation; only readonly_mirror.

pub mod cursor_agent;
pub mod doctor;
pub mod history;
pub mod meta;

use anyhow::{Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct AskRequest {
    pub project_root: PathBuf,
    pub topic: String,
    pub prompt_file: PathBuf,
    pub models: Option<Vec<String>>,
    /// SPEC §6.3 / §9: explicit profile chain for this run (comma-
    /// separated CLI flag). `None` ⇒ single-element chain consisting
    /// of the resolved default profile (SQLite `meta.default_profile`,
    /// else literal `"default"`, else first registered profile).
    pub profiles: Option<Vec<String>>,
    pub dry_run: bool,
    pub budget_only: bool,
    /// When `true`, suppress the readonly directive a2a otherwise
    /// prepends to the prompt. SPEC §8.2.
    pub no_readonly_prefix: bool,
    /// SPEC §8.0: cursor-agent `--mode` (validated by clap to be
    /// `"agent"` or `"plan"`). `None` → per-alias fallback to the
    /// `default_mode` column on the SQLite `model_aliases` row, then
    /// literal `"agent"`.
    pub mode: Option<String>,
    /// SPEC §8.0: optional `--sandbox <enabled|disabled>` passthrough.
    pub sandbox: Option<String>,
    /// SPEC §14: when `true`, attach a `[models.budget]` sub-table
    /// to each successful model's row in this run's `meta.toml`.
    /// Default `false`.
    pub log_budget: bool,
}

pub fn ask_orchestrator(req: AskRequest) -> Result<()> {
    let frontmatter = crate::prompt::parse_frontmatter(&req.prompt_file)?;

    // SPEC §11: when `--models` is omitted, run only the alias that
    // was added to the SQLite `model_aliases` table FIRST (lowest
    // `created_at`). If the table is empty we bail with a clear
    // remediation hint — there is no "tool-default" alias set.
    let store_for_aliases = crate::auth::store::open()?;
    let raw_models: Vec<String> = match req.models.clone() {
        Some(v) if !v.is_empty() => v,
        _ => {
            let aliases = store_for_aliases.list_model_aliases()?;
            match aliases.first() {
                Some(first) => vec![first.alias.clone()],
                None => {
                    return Err(anyhow::Error::new(crate::UserError(
                        "no model aliases configured. Run \
                         `a2a models add <alias> --model <cursor-id>` to register one, \
                         then re-run this command (with or without `--models`)."
                            .to_string(),
                    )));
                }
            }
        }
    };
    drop(store_for_aliases);

    // Deduplicate aliases, validate names.
    //
    // alias and topic have different rule sets:
    //   - alias must round-trip through `a2a models add`, which rejects
    //     `(`/`)` (validate_alias_strict) → the same here so a user
    //     can't pass `--models 'foo(1)'` and get the misleading
    //     "unknown alias" instead of "invalid character".
    //   - topic is just used as a directory-name fragment, so the
    //     looser rule (alphanumeric + `-_()`) is fine and lets
    //     consult_dir names mirror profile-style suffixes.
    let mut models: Vec<String> = Vec::with_capacity(raw_models.len());
    for m in raw_models {
        if !validate_alias_strict(&m) {
            crate::user_bail!(
                "model alias must be alphanumeric / '-' / '_' (got '{m}'). \
                 Run `a2a models list` to see configured aliases."
            );
        }
        if !models.contains(&m) {
            models.push(m);
        } else {
            tracing::warn!("model alias '{m}' specified multiple times; deduplicated");
        }
    }
    if !validate_topic_slug(&req.topic) {
        crate::user_bail!("topic slug must be alphanumeric / '-' / '_' / '(' / ')': {}", req.topic);
    }

    // SPEC §11: `output_root` is hardcoded `"consultations"` (no more
    // [defaults] override). Defensively still run the symlink-escape
    // canonicalization in case a user has made `consultations/` (or
    // any ancestor) a junction pointing outside the project.
    let output_root = crate::defaults::OUTPUT_ROOT;
    validate_project_relative(output_root, "consultations output root")?;
    let canonical_project = req
        .project_root
        .canonicalize()
        .with_context(|| format!("canonicalize project root {}", req.project_root.display()))?;
    let intended_output_root = canonical_project.join(output_root);
    let canon_anchor = deepest_canonical_ancestor(&intended_output_root)?;
    if !canon_anchor.starts_with(&canonical_project) {
        crate::user_bail!(
            "consultations output root resolves to {} which escapes the project root {} \
             (an existing ancestor is a symlink/junction pointing outside the project). \
             Refusing to write outside the project.",
            canon_anchor.display(),
            canonical_project.display()
        );
    }

    let now = Utc::now();
    let suffix: String = uuid::Uuid::new_v4().simple().to_string().chars().take(6).collect();
    let dir_name = format!(
        "{}-{:03}-{}-{}",
        now.format("%Y%m%d-%H%M%S"),
        now.timestamp_subsec_millis(),
        req.topic,
        suffix
    );
    let consult_dir = req.project_root.join(output_root).join(&dir_name);

    // SPEC §15: housekeeping. Spawn a detached background thread that
    // prunes consultation dirs older than 7 days. Uses `std::thread`
    // (not `tokio::spawn`) so it survives the tokio runtime drop at
    // the end of this `ask` invocation; the thread is best-effort and
    // any failures are logged via `tracing::warn!` only. Skipped in
    // dry_run / budget_only modes since neither path commits to the
    // on-disk consultations tree.
    if !req.dry_run && !req.budget_only {
        let consult_root = req.project_root.join(output_root);
        const RETAIN_DAYS: u64 = 7;
        history::spawn_housekeep_old_consults(consult_root, RETAIN_DAYS);
    }

    // For --dry-run AND --budget-only, never create the on-disk
    // consultation dir (no answers will be written there). Use a
    // throwaway tempdir as the workspace anchor so downstream code
    // that expects `consult_dir` to exist doesn't break.
    let no_persist = req.dry_run || req.budget_only;
    let _dryrun_tmp_guard = if !no_persist {
        std::fs::create_dir_all(&consult_dir)
            .with_context(|| format!("create {}", consult_dir.display()))?;
        let dst_prompt = consult_dir.join("prompt.md");
        std::fs::copy(&req.prompt_file, &dst_prompt)
            .with_context(|| format!("copy prompt -> {}", dst_prompt.display()))?;
        None
    } else {
        let td = tempfile::tempdir().context("create throwaway tempdir")?;
        Some(td)
    };
    let effective_consult_dir = match &_dryrun_tmp_guard {
        Some(td) => td.path().to_path_buf(),
        None => consult_dir.clone(),
    };

    println!("Topic:      {}", req.topic);
    println!("Models:     {:?}", models);
    println!(
        "Mode:       {}",
        req.mode.as_deref().unwrap_or("(per-alias default_mode)")
    );
    println!("Prompt:     {}", req.prompt_file.display());
    if req.dry_run {
        println!("Output dir: <dry-run; no files written>");
    } else if req.budget_only {
        println!("Output dir: <budget-only; no files written>");
    } else {
        println!("Output dir: {}", consult_dir.display());
    }
    println!();

    // Resolve the model alias rows (used by both budget_only and the
    // real ask path). Bails early on unknown alias before we spend
    // tokio bring-up time.
    let mut alias_rows: Vec<crate::auth::store::ModelAlias> = Vec::with_capacity(models.len());
    {
        let store = crate::auth::store::open()?;
        for alias in &models {
            let row = store.get_model_alias(alias)?.ok_or_else(|| {
                anyhow::Error::new(crate::UserError(format!(
                    "unknown model alias: {alias} (run `a2a models list` to see registered \
                     aliases; run `a2a models add {alias} --model <cursor-id>` to register one)"
                )))
            })?;
            if row.cursor_model.trim().is_empty() {
                crate::user_bail!(
                    "model alias '{alias}' has empty cursor_model — re-add it with \
                     `a2a models add {alias} --model <cursor-id> --force`"
                );
            }
            alias_rows.push(row);
        }
    }

    if req.budget_only {
        return budget_only_print(&req, &frontmatter, &alias_rows);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("init tokio runtime")?;

    runtime.block_on(async move {
        // SPEC §6.3 / §9: Build the profile chain from the explicit
        // CLI `--profiles a,b,c` flag. When omitted, fall back to a
        // single-element chain consisting of the resolved default
        // profile (SQLite `meta.default_profile` ← `a2a auth use`,
        // else the literal `"default"` profile if it exists, else the
        // first profile by `created_at`). All cross-account fallback
        // is now CLI-time explicit; there is no persistent
        // `[fallback] default_chain` or per-alias `fallback_profiles`.
        let chain: Vec<String> = match req.profiles.as_ref() {
            Some(list) if !list.is_empty() => {
                let mut deduped = Vec::with_capacity(list.len());
                for p in list {
                    if !deduped.contains(p) {
                        deduped.push(p.clone());
                    }
                }
                deduped
            }
            _ => vec![resolve_default_profile()?],
        };

        // Cross-task signal: set to true when KeyDead empties the
        // credentials store. All in-flight alias tasks check this flag
        // at every chain step and during transient backoff to bail
        // fast instead of wasting more cursor-agent calls; the
        // orchestrator below uses it to decide whether to print the
        // credentials-drained banner and exit BusinessFailure.
        let store_drained = std::sync::Arc::new(
            std::sync::atomic::AtomicBool::new(false),
        );

        let mut tasks: Vec<SingleModelTask> = Vec::with_capacity(alias_rows.len());
        for row in alias_rows {
            // SPEC §8.0: effective mode resolution order
            //   1. CLI `--mode` (req.mode) — explicit override across all aliases
            //   2. row.default_mode — per-alias default from SQLite
            //   3. literal "agent" — cursor-agent's implicit default
            let effective_mode = req
                .mode
                .clone()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    let m = row.default_mode.trim();
                    if m.is_empty() { None } else { Some(m.to_string()) }
                })
                .unwrap_or_else(|| "agent".to_string());
            tasks.push(SingleModelTask {
                topic: req.topic.clone(),
                alias: row.alias.clone(),
                model_alias: row,
                chain: chain.clone(),
                consult_dir: effective_consult_dir.clone(),
                prompt_file: req.prompt_file.clone(),
                frontmatter: frontmatter.clone(),
                dry_run: req.dry_run,
                project_root: req.project_root.clone(),
                no_readonly_prefix: req.no_readonly_prefix,
                mode: effective_mode,
                sandbox: req.sandbox.clone(),
                log_budget: req.log_budget,
                store_drained: store_drained.clone(),
            });
        }

        let mut succeeded = 0;
        let mut failed = 0;
        if crate::defaults::PARALLEL {
            let stagger = std::time::Duration::from_secs(crate::defaults::STAGGER_SECS);
            let total_models = tasks.len();
            let mut handles = Vec::with_capacity(total_models);
            for (i, task) in tasks.into_iter().enumerate() {
                let alias = task.alias.clone();
                handles.push(tokio::spawn(async move { (alias, task.execute().await) }));
                if !stagger.is_zero() && i + 1 < total_models {
                    tokio::time::sleep(stagger).await;
                }
            }
            // Drain ALL handles even if one panics.
            for h in handles {
                match h.await {
                    Ok((alias, Ok(()))) => {
                        crate::pln!("[{alias}] ok");
                        succeeded += 1;
                    }
                    Ok((alias, Err(e))) => {
                        crate::pln!("[{alias}] failed: {e:#}");
                        failed += 1;
                    }
                    Err(join_err) => {
                        crate::pln!("[<unknown>] task panicked: {join_err}");
                        failed += 1;
                    }
                }
            }
        } else {
            for task in tasks {
                let alias = task.alias.clone();
                match task.execute().await {
                    Ok(()) => {
                        crate::pln!("[{alias}] ok");
                        succeeded += 1;
                    }
                    Err(e) => {
                        crate::pln!("[{alias}] failed: {e:#}");
                        failed += 1;
                    }
                }
            }
        }
        println!();
        crate::pln!("Done. {succeeded} succeeded / {failed} failed.");
        if !req.dry_run {
            println!("Inspect raw answers in: {}", consult_dir.display());
        }

        // KeyDead-drain-during-run: at least one alias hit a KeyDead
        // that emptied `~/.a2a/credentials.db`. Tell the user clearly
        // what happened, why we aborted the rest, and how to recover —
        // do this *before* the generic "all failed" branch so the
        // surfaced error is the actual cause, not a generic summary.
        if store_drained.load(std::sync::atomic::Ordering::Relaxed) {
            println!();
            println!("==============================================================");
            println!("  No credentials left.");
            println!();
            println!("  At least one model alias deleted its profile via the SPEC §6.3");
            println!("  KeyDead path (e.g. 401 / unpaid invoice / quota exceeded), and");
            println!("  no profiles remain in `~/.a2a/credentials.db`. The rest of the");
            println!("  alias tasks were aborted instead of issuing more cursor-agent");
            println!("  calls that cannot authenticate.");
            println!();
            println!("  To recover:");
            println!("    1. Resolve the upstream account problem reported above");
            println!("       (pay invoice / restore account / etc).");
            println!("    2. `a2a auth add <name> [--from-stdin]` to register an API key.");
            println!("    3. `a2a auth use <name>` to set it as default.");
            println!("    4. Re-run your `a2a ask ...` command.");
            println!("==============================================================");
            return Err(crate::business_failure(
                "credentials store drained mid-run; no profiles remain. Re-add a profile and re-run.".to_string()
            ));
        }

        if failed > 0 && succeeded == 0 {
            return Err(crate::business_failure(format!(
                "all {failed} model task(s) failed; see {} for details",
                consult_dir.display()
            )));
        }
        if failed > 0 {
            println!(
                "warning: {failed} of {} model task(s) failed; synthesis should note missing answers",
                succeeded + failed
            );
        }
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

/// Topic slugs are used as a directory-name fragment under
/// `consultations/<ts>-<topic>-<uuid>/`. Allow `( )` so users can pick
/// topic names that mirror profile naming conventions.
fn validate_topic_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '(' || c == ')'
        })
}

/// Model aliases must round-trip through `a2a models add` (which
/// rejects `( )`). Tightening the `--models` validation here keeps
/// the error message accurate ("must be alphanumeric / '-' / '_'")
/// and points the user at the real problem before the chain runs and
/// a generic "unknown alias" surfaces.
fn validate_alias_strict(alias: &str) -> bool {
    !alias.is_empty()
        && alias
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn budget_only_print(
    req: &AskRequest,
    frontmatter: &crate::prompt::Frontmatter,
    alias_rows: &[crate::auth::store::ModelAlias],
) -> Result<()> {
    let prompt_body_chars = std::fs::read_to_string(&req.prompt_file)
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let context_chars: usize = frontmatter
        .context_files
        .iter()
        .map(|p| count_chars_in_entry(&req.project_root, p))
        .sum();
    let directive_full = crate::fallback::READONLY_DIRECTIVE.chars().count() + 2;

    // Compute per-alias effective mode and whether the readonly
    // directive would be auto-prepended (mirrors fallback runner's
    // `suppress_readonly = no_readonly_prefix || mode == "plan"`).
    // SPEC §11: no `always_include` — the entire context surface
    // comes from the prompt's frontmatter `context_files`.
    let mut total = 0usize;
    let mut any_directive = false;
    let mut all_directive = true;
    for row in alias_rows {
        let effective_mode = req
            .mode
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                let m = row.default_mode.trim();
                if m.is_empty() { None } else { Some(m.to_string()) }
            })
            .unwrap_or_else(|| "agent".to_string());
        let inject_directive = !req.no_readonly_prefix && effective_mode != "plan";
        let dchars = if inject_directive { directive_full } else { 0 };
        total += prompt_body_chars + dchars + context_chars;
        if inject_directive {
            any_directive = true;
        } else {
            all_directive = false;
        }
    }

    let aliases: Vec<&str> = alias_rows.iter().map(|r| r.alias.as_str()).collect();
    println!("--budget-only (character counts as a token proxy):");
    println!("  prompt body         = {prompt_body_chars} chars");
    let directive_note = match (any_directive, all_directive) {
        (true, true) => format!("{directive_full} chars (auto-prepended for every alias)"),
        (true, false) => format!(
            "{directive_full} chars (auto-prepended only for alias(es) running in `agent` mode)"
        ),
        (false, _) => "0 chars (suppressed: --no-readonly-prefix or all aliases run in `plan` mode)".into(),
    };
    println!("  readonly directive  = {directive_note}");
    println!("  declared context    = {context_chars} chars");
    println!("  models to spawn     = {} ({:?})", aliases.len(), aliases);
    println!("  total input ≈         {total} chars across all models");
    println!();
    println!("(This is a static estimate. Actual cursor-agent quota use may differ.)");
    Ok(())
}

/// SPEC §6.3 / §11: resolve the default profile name when `--profiles`
/// is omitted. Three-tier fallback:
///   1. SQLite `meta.default_profile` (set by `a2a auth use`).
///   2. The literal profile named `"default"` if it exists.
///   3. The first profile by `created_at` (i.e. the earliest registered).
///
/// Bails when there are no profiles at all (`a2a auth add` first).
fn resolve_default_profile() -> Result<String> {
    let store = crate::auth::store::open()?;
    if let Some(p) = store.get_default_profile()?
        && store.profile_exists(&p)?
    {
        return Ok(p);
    }
    if store.profile_exists("default")? {
        return Ok("default".to_string());
    }
    let mut profiles = store.list_profiles()?;
    profiles.sort_by_key(|p| p.created_at);
    match profiles.into_iter().next() {
        Some(p) => Ok(p.name),
        None => Err(anyhow::Error::new(crate::UserError(
            "no profiles registered in `~/.a2a/credentials.db`. \
             Run `a2a auth add` to register an API key first."
                .to_string(),
        ))),
    }
}

/// Count chars in a context entry (recursive for directories,
/// matching what readonly_mirror copies). Cap per-file at 4 MiB
/// so a multi-hundred-MB attachment doesn't OOM `--budget-only`.
pub(crate) fn count_chars_in_entry(project_root: &Path, rel: &str) -> usize {
    let path_obj = std::path::Path::new(rel);
    if path_obj.is_absolute() {
        return 0;
    }
    if path_obj
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return 0;
    }
    let p = project_root.join(rel);
    let canonical_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    // Canonicalize failure (e.g. permission denied on a parent) must
    // not silently bypass the bounds check. Treat un-canonicalizable
    // paths as "skip" (return 0) so the budget estimate matches what
    // `readonly_mirror` would actually copy (which also skips
    // entries it can't canonicalize).
    let canonical_p = match p.canonicalize() {
        Ok(c) => c,
        Err(_) => return 0,
    };
    if !canonical_p.starts_with(&canonical_root) {
        return 0;
    }
    if p.is_file() {
        return read_one_for_budget(&p);
    }
    if !p.is_dir() {
        return 0;
    }
    const SKIP: [&str; 5] = ["target", "node_modules", ".git", ".a2a", "consultations"];
    let mut total = 0usize;
    for entry in walkdir::WalkDir::new(&p).into_iter().filter_entry(|e| {
        e.depth() == 0
            || !e.file_type().is_dir()
            || !SKIP.contains(&e.file_name().to_string_lossy().as_ref())
    }) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let ft = entry.file_type();
        if !ft.is_file() && !ft.is_symlink() {
            continue;
        }
        if ft.is_symlink() {
            match std::fs::canonicalize(entry.path()) {
                Ok(canon) if canon.starts_with(&canonical_root) => {}
                _ => continue,
            }
        }
        total += read_one_for_budget(entry.path());
    }
    total
}

fn read_one_for_budget(p: &Path) -> usize {
    const MAX_BUDGET_FILE_BYTES: u64 = 4 * 1024 * 1024;
    if let Ok(meta) = std::fs::metadata(p)
        && meta.len() > MAX_BUDGET_FILE_BYTES
    {
        tracing::warn!(
            "count_chars: {} is {} bytes (>{} MiB); skipping for budget estimate",
            p.display(),
            meta.len(),
            MAX_BUDGET_FILE_BYTES / 1024 / 1024
        );
        return 0;
    }
    std::fs::read_to_string(p)
        .map(|s| s.chars().count())
        .unwrap_or(0)
}

/// Reject path-shaped config values that would let a malicious
/// `.a2a/config.toml` escape the project directory. Allows
/// `consultations/`, `nested/dir/`, etc.; rejects absolute paths,
/// parent traversal, Windows path prefixes, empty.
pub(crate) fn validate_project_relative(value: &str, what: &str) -> Result<()> {
    if value.is_empty() {
        crate::user_bail!("{what} cannot be empty");
    }
    let p = std::path::Path::new(value);
    if p.is_absolute() {
        crate::user_bail!("{what} must be a project-relative path (got absolute: '{value}')");
    }
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                crate::user_bail!(
                    "{what} must not contain parent-directory traversal (got '{value}')"
                );
            }
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                crate::user_bail!("{what} must be a project-relative path (got '{value}')");
            }
            _ => {}
        }
    }
    Ok(())
}

/// Walk `path` from leaf toward root, returning the first ancestor
/// that successfully canonicalizes. The leaf may not exist yet (we're
/// about to `create_dir_all` it), but each ancestor is checked so
/// symlink/junction escapes through any intermediate path component
/// are caught. The walk always terminates: if nothing else exists,
/// the filesystem root is canonicalizable.
pub(crate) fn deepest_canonical_ancestor(path: &Path) -> Result<PathBuf> {
    let mut cur: Option<&Path> = Some(path);
    while let Some(p) = cur {
        if let Ok(canon) = p.canonicalize() {
            return Ok(canon);
        }
        cur = p.parent();
    }
    crate::user_bail!(
        "could not canonicalize any ancestor of {} (filesystem in unexpected state)",
        path.display()
    );
}

struct SingleModelTask {
    topic: String,
    alias: String,
    model_alias: crate::auth::store::ModelAlias,
    /// SPEC §6.3 / §9: profile fallback chain for this run. Owned per
    /// task because the runner uses it as `Vec<String>` directly; all
    /// tasks share the same chain (resolved once in the orchestrator
    /// from `--profiles` or the default-profile fallback).
    chain: Vec<String>,
    consult_dir: PathBuf,
    prompt_file: PathBuf,
    frontmatter: crate::prompt::Frontmatter,
    dry_run: bool,
    project_root: PathBuf,
    no_readonly_prefix: bool,
    mode: String,
    sandbox: Option<String>,
    log_budget: bool,
    store_drained: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl SingleModelTask {
    async fn execute(self) -> Result<()> {
        crate::fallback::run_with_fallback(
            &self.topic,
            &self.alias,
            &self.model_alias,
            self.chain,
            &self.consult_dir,
            &self.prompt_file,
            &self.frontmatter,
            &self.project_root,
            self.dry_run,
            self.no_readonly_prefix,
            &self.mode,
            self.sandbox.as_deref(),
            self.log_budget,
            self.store_drained,
        )
        .await
    }
}
