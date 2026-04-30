// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! Profile fallback chain + 3-class error routing. SPEC §6 / §9.
//!
//! Three error classes (hard-coded keyword match, no toml config):
//!   - KeyDead: account-level failure (401, billing, quota). Delete profile, try next.
//!   - ModelUnavailable: this model not available on this account. Skip alias.
//!   - Transient: network / rate-limit. Retry same profile up to 3 times, then skip alias.
//!   - Unknown: treat like ModelUnavailable (skip alias, no fallback).
//!
//! No regex pattern config, no disable_temp / disable_perm state, no
//! per-pattern retry budget, no error.md file. All failure context is
//! printed to the command line — SPEC §6.4 / §13.

use crate::auth;
use crate::auth::store::ModelAlias;
use crate::isolation;
use crate::prompt::Frontmatter;
use crate::runner::cursor_agent::{self, CursorAgentSpec};
use crate::runner::meta::{BudgetInfo, FallbackAttempt, ModelMeta, append_model_meta};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Sentinel error message used when an alias bails because the
/// credentials store became empty mid-run (or another alias already
/// drained it). The orchestrator looks for this exact string when
/// deciding whether to print the credentials-drained banner.
pub const STORE_DRAINED_MSG: &str = "credentials store is empty";

/// SPEC §8.4 / §8.5: short English continuation sentence sent in
/// place of (or in front of) the original prompt when continuing an
/// interrupted session. Hardcoded; not localised. Cursor backend
/// models all handle multilingual prompts well; English keeps token
/// cost minimal and avoids adding a directive_lang config.
pub const RESUME_CONTINUATION_PROMPT: &str =
    "Sorry, the previous session was interrupted unexpectedly. Please continue your work.";

/// Prefix block separator used when SPEC §8.5 Step 2 reconstructs a
/// fresh prompt with continuation notice + original prompt below.
const STEP2_ORIGINAL_PROMPT_BANNER: &str =
    "====================  Original prompt below  ====================";

/// SPEC §8.1: prepended to every prompt unless `--no-readonly-prefix` is set.
pub(crate) const READONLY_DIRECTIVE: &str = "\
====================  a2a read-only directive  ====================
You are running inside an isolated workspace mirror provided by a2a.
You MUST follow these constraints (please respect them so the user's
project source stays untouched):

1. DO NOT modify any existing file in this workspace.
2. **Your final answer / synthesis / report MUST be printed to stdout
   as your normal model response.**  This is how a2a captures the
   deliverable and shows it to the user.  Do not try to defer the
   answer into a file — your stdout reply IS the deliverable.
3. If your environment also lets you create files, you may
   ADDITIONALLY drop supporting artifacts inside `.a2a/`
   (e.g. `.a2a/notes.md`). The stdout answer is still primary;
   the `.a2a/` files are optional context.
4. NEVER write outside the workspace root. NEVER touch paths starting
   with `..` or absolute paths to system locations.

If your task is a code review and you considered a fix you did not
apply, describe the diff inline in your stdout reply (it is a markdown
response — code blocks are fine).

====================  user prompt below  ==========================
";

/// SPEC §10: redirect text used when the prompt is too big to pass on
/// the cmdline (Windows 32K cap) or contains non-ASCII (Windows
/// PowerShell encoding mojibake).
pub(crate) const INDIRECT_PROMPT_REDIRECT: &str = "\
====================  a2a indirect-prompt mode  ====================
The actual task description for this run was too large (or used
non-ASCII characters) to fit safely in the operating-system command
line, so a2a wrote it to a file in your workspace instead.

YOUR FIRST ACTION MUST BE: read the file `.a2a/_prompt.md` (relative
to your workspace root) using your file-read tool. That file is the
canonical, authoritative prompt.

Constraints (also restated inside `_prompt.md`):
- Do NOT edit `.a2a/_prompt.md`.
- Print your final answer to stdout as your normal model response.
  Do not write the answer to any file under `.a2a/`.

After reading `.a2a/_prompt.md`, follow its instructions exactly as
if it had been the original prompt. Begin now by reading the file.
====================================================================
";

/// SPEC §6.1: three-way classification of a cursor-agent failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorClass {
    /// API key-level problem: 401, billing required, quota exceeded.
    /// Profile is dead; delete it and try the next one in the chain.
    KeyDead,
    /// This specific model is not available on this account. Skip
    /// the model alias entirely (no fallback chain helps).
    ModelUnavailable,
    /// Network / rate-limit. Retry same profile.
    Transient,
    /// Unrecognised. Treat as ModelUnavailable (skip alias).
    Unknown,
}

/// Returns `true` iff `tok` appears in `s` as an independent token —
/// i.e. surrounded by characters that are not ASCII alphanumeric (or
/// at the string boundary). SPEC §6.2 specifies `401` / `429` as
/// "independent tokens" so substrings like `4019` / `4290` don't match
/// while real-world phrasings like `"status 401"`, `"(401)"`, `"401\n"`
/// all do.
fn contains_token(s: &str, tok: &str) -> bool {
    s.match_indices(tok).any(|(i, _)| {
        let before = s[..i].chars().next_back();
        let after = s[i + tok.len()..].chars().next();
        before.is_none_or(|c| !c.is_ascii_alphanumeric())
            && after.is_none_or(|c| !c.is_ascii_alphanumeric())
    })
}

/// SPEC §6.2: classify based on stderr keyword match (case-insensitive).
/// Stdout is intentionally NOT consulted — model output text could
/// otherwise hijack routing decisions.
fn classify(stderr: &str) -> ErrorClass {
    let s = stderr.to_lowercase();
    // KeyDead — account-level (401 / billing / quota). Substring
    // keywords for natural-language phrases; token-boundary check
    // for the bare `401` numeric.
    let key_dead_substrings = [
        "unauthorized",
        "invalid api key",
        "authentication failed",
        "billing required",
        "payment required",
        "payment overdue",
        "subscription expired",
        "unpaid invoice",
        "quota exceeded",
        "out of credits",
        "monthly limit",
        "usage limit",
        "insufficient credits",
    ];
    if key_dead_substrings.iter().any(|k| s.contains(k)) || contains_token(&s, "401") {
        return ErrorClass::KeyDead;
    }
    // ModelUnavailable — model-level.
    let model_unavail_keywords = [
        "model not available",
        "no access to model",
        "cannot use this model",
        "model access denied",
    ];
    if model_unavail_keywords.iter().any(|k| s.contains(k)) {
        return ErrorClass::ModelUnavailable;
    }
    // Transient — network / rate-limit. `429` uses token boundary
    // (so `4290` / `1429` don't false-match); other phrases stay as
    // substring matches.
    let transient_substrings = [
        "rate limit",
        "too many requests",
        "timed out",
        "timeout",
        "connection refused",
        "connection reset",
        "tls",
        "dns",
        "failed to reach the cursor api",
    ];
    if transient_substrings.iter().any(|k| s.contains(k)) || contains_token(&s, "429") {
        return ErrorClass::Transient;
    }
    ErrorClass::Unknown
}

/// SPEC §8.4 / §8.5: state machine for the per-profile call loop.
/// Drives prompt construction and `--resume` flag selection on each
/// cursor-agent invocation within a single profile attempt.
#[derive(Debug, Clone)]
enum CallState {
    /// First call for this profile, no prior session to resume.
    /// Prompt = full `prompt_text` (with READONLY_DIRECTIVE if not
    /// suppressed). resume = None.
    Fresh,
    /// SPEC §8.5: KeyDead path advanced to a new profile (different
    /// account). Cursor backend silently drops history on
    /// cross-account `--resume`, so we skip the probe and go fresh
    /// with a continuation prefix. Prompt = directive + short
    /// continuation sentence + banner + original. resume = None.
    FreshWithContinuationPrefix,
    /// SPEC §8.4: transient retry — same profile, captured session_id
    /// from this profile's own previous attempt. Prompt = short
    /// continuation sentence. resume = captured session_id.
    TransientResume(String),
}

impl CallState {
    fn resume_session_id(&self) -> Option<String> {
        match self {
            Self::TransientResume(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// One-shot human-readable phase tag for progress logging.
    fn phase_tag(&self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::FreshWithContinuationPrefix => "fresh with continuation prefix",
            Self::TransientResume(_) => "resume after transient",
        }
    }
}

/// Build the prompt body for a given `CallState`. `user_prompt` is
/// the user's untouched prompt body (the `_prompt.md` source); the
/// READONLY_DIRECTIVE injection logic mirrors the entry-point
/// suppression rule (SPEC §8.2).
fn build_prompt_for_state(
    state: &CallState,
    suppress_readonly: bool,
    user_prompt: &str,
    full_prompt_with_directive: &str,
) -> String {
    match state {
        CallState::Fresh => full_prompt_with_directive.to_string(),
        CallState::TransientResume(_) => RESUME_CONTINUATION_PROMPT.to_string(),
        CallState::FreshWithContinuationPrefix => {
            if suppress_readonly {
                format!(
                    "{RESUME_CONTINUATION_PROMPT}\n\n{STEP2_ORIGINAL_PROMPT_BANNER}\n\n{user_prompt}"
                )
            } else {
                format!(
                    "{READONLY_DIRECTIVE}\n\n{RESUME_CONTINUATION_PROMPT}\n\n{STEP2_ORIGINAL_PROMPT_BANNER}\n\n{user_prompt}"
                )
            }
        }
    }
}

/// Either return the prompt body unchanged (direct path) or write it
/// to `<workspace>/.a2a/_prompt.md` and return the indirect redirect
/// string (SPEC §10.2). Mirrors the original write site so retries
/// after state transitions can re-write the file with the new body.
fn materialise_prompt(
    workspace: &isolation::IsolatedWorkspace,
    body: &str,
    use_indirect: bool,
) -> Result<String> {
    if !use_indirect {
        return Ok(body.to_string());
    }
    workspace.assert_alive()?;
    let a2a_dir = workspace.root().join(".a2a");
    if !a2a_dir.exists() {
        std::fs::create_dir(&a2a_dir).with_context(|| {
            format!(
                "create {} (workspace root must already exist)",
                a2a_dir.display()
            )
        })?;
    }
    let indirect_path = a2a_dir.join("_prompt.md");
    std::fs::write(&indirect_path, body)
        .with_context(|| format!("write indirect prompt {}", indirect_path.display()))?;
    Ok(INDIRECT_PROMPT_REDIRECT.to_string())
}

/// SPEC §6.3 fallback runner. Walk the profile chain; classify
/// failures; retry / delete / skip per class.
#[allow(clippy::too_many_arguments)]
pub async fn run_with_fallback(
    topic: &str,
    alias: &str,
    model_cfg: &ModelAlias,
    chain: Vec<String>,
    consult_dir: &Path,
    prompt_file: &Path,
    frontmatter: &Frontmatter,
    project_root: &Path,
    dry_run: bool,
    no_readonly_prefix: bool,
    mode: &str,
    sandbox: Option<&str>,
    log_budget: bool,
    store_drained: Arc<AtomicBool>,
) -> Result<()> {
    // SPEC §11: the indirect-prompt threshold is hardcoded. Bind it
    // locally so the inner state-machine loop can re-evaluate
    // against the current body length without re-fetching.
    let inline_prompt_max_bytes = crate::defaults::INLINE_PROMPT_MAX_BYTES;
    let user_prompt = cursor_agent::read_prompt_text(prompt_file)?;
    // SPEC §8.1: suppress READONLY_DIRECTIVE auto-injection when:
    //   1. user explicitly passed `--no-readonly-prefix`, OR
    //   2. mode is `plan` — cursor-agent's plan mode is already read-only
    //      at the binary level, so injecting "DO NOT modify any file"
    //      is redundant noise that consumes prompt tokens for no gain.
    let suppress_readonly = no_readonly_prefix || mode == "plan";
    let prompt_text = if suppress_readonly {
        user_prompt.clone()
    } else {
        format!("{}\n\n{}", READONLY_DIRECTIVE, user_prompt)
    };
    // SPEC §10.1: indirect-prompt triggers on size OR on Windows +
    // non-ASCII. The decision is **re-evaluated per cursor-agent
    // call** inside the inner state-machine loop because state
    // transitions (Fresh → FreshWithContinuationPrefix) add ~200
    // bytes of continuation prefix and may push a borderline-size
    // prompt over the threshold.
    let original_prompt_chars = prompt_text.chars().count();
    let answer_path = consult_dir.join(format!("{alias}.answer.md"));

    let mut attempts: Vec<FallbackAttempt> = Vec::new();
    let started_total = Instant::now();
    let mut db = auth::store::open()?;

    // SPEC §14.4: per-alias session_id ledger. `last_session_id`
    // tracks the most recent cursor-agent invocation's id (used both
    // as the resume target for the next call AND as the value
    // recorded to meta.toml). `session_ids` accumulates every
    // distinct id seen across all profile attempts.
    let mut last_session_id: Option<String> = None;
    let mut session_ids: Vec<String> = Vec::new();

    // If a peer alias already drained the store before we even
    // started (e.g. KeyDead on the shared `default` profile from
    // alias #1 deleted it), bail right away — no point spawning a
    // cursor-agent call we know cannot authenticate. SPEC §14
    // requires a row per alias in meta.toml even on failure, but at
    // *this* point `attempts` is provably empty (we haven't entered
    // the chain loop yet), so a row written here would be content-
    // free and just adds noise. Keep it as a bare `bail!`; the
    // four drain checkpoints inside the loop below DO write meta.
    if store_drained.load(Ordering::Relaxed) {
        bail!("aborted: {STORE_DRAINED_MSG} (deleted by a peer alias's KeyDead handler)");
    }
    // Likewise, if the store was already empty when this run began
    // (user removed the last profile, or KeyDead from a previous run
    // emptied it). dry_run is allowed through so users can still
    // preview the cursor-agent command without an active profile.
    //
    // SPEC §6.3 store-drained banner is for **KeyDead-induced drain
    // mid-run** — i.e. some account had a KeyDead error and a2a
    // deleted the profile. If the store was already empty at fallback
    // entry, no profile was deleted, no upstream account problem
    // happened, and the SPEC §6.3 banner would mis-describe the
    // cause. So bail this alias only — do **not** raise the shared
    // `store_drained` signal. The orchestrator's generic "all
    // failed" BusinessFailure path will surface the per-alias
    // "no profiles registered" error.
    if !dry_run && db.is_empty()? {
        bail!(
            "no profiles registered in `~/.a2a/credentials.db`; run `a2a auth add <name> [--from-stdin]` first"
        );
    }

    // SPEC §14: every alias must leave a row in meta.toml, even when
    // it bails because a peer alias drained the store mid-run. The
    // four drain checkpoints inside the chain loop run AFTER the
    // `attempts` vector may already hold rows from prior chain steps
    // (profile not_found, KeyDead-deleted, transient retries that
    // got partway, etc.), so a bare `bail!` would discard that
    // forensic data. Wrap them in a closure that writes meta first
    // and then bails. (The pre-chain entry check above runs with
    // `attempts` empty, so it stays as a bare bail.)
    macro_rules! bail_drain_with_meta {
        ($msg:expr) => {{
            if let Err(e) = write_failure_meta(
                consult_dir,
                topic,
                alias,
                model_cfg,
                mode,
                &chain,
                attempts.clone(),
                started_total.elapsed().as_millis() as u64,
                session_ids.clone(),
                last_session_id.clone(),
            )
            .await
            {
                tracing::warn!(
                    "could not write meta.toml failure record (the user-facing error below is the real cause): {e:#}"
                );
            }
            bail!($msg);
        }};
    }

    for (idx, profile_name) in chain.iter().enumerate() {
        if store_drained.load(Ordering::Relaxed) {
            bail_drain_with_meta!(format!(
                "aborted: {STORE_DRAINED_MSG} (deleted by a peer alias's KeyDead handler)"
            ));
        }
        let profile_opt = db.get_profile(profile_name)?;

        if dry_run {
            // Synthetic preview; never touch the workspace.
            let synth = project_root.join(".a2a/dry-run-workspace");
            let spec = CursorAgentSpec {
                cursor_model: model_cfg.cursor_model.clone(),
                mode: mode.to_string(),
                workspace: synth,
                prompt_text: prompt_text.clone(),
                api_key: String::new(),
                sandbox: sandbox.map(str::to_string),
                resume_session_id: None,
            };
            let status = if profile_opt.is_some() {
                "ok"
            } else {
                "<NOT FOUND>"
            };
            crate::pln!(
                "[{alias}] profile={profile_name} ({status}) → DRY-RUN: {}",
                cursor_agent::dry_run_command_string(&spec)
            );
            attempts.push(FallbackAttempt {
                profile: profile_name.clone(),
                success: true,
                error_class: None,
                error_excerpt: None,
                elapsed_ms: 0,
                session_id: None,
            });
            return Ok(());
        }

        let profile = match profile_opt {
            Some(p) => p,
            None => {
                let msg = format!("profile '{profile_name}' not found");
                crate::pln!("[{alias}] profile={profile_name} → SKIPPED ({msg})");
                attempts.push(FallbackAttempt {
                    profile: profile_name.clone(),
                    success: false,
                    error_class: Some("not_found".into()),
                    error_excerpt: Some(msg),
                    elapsed_ms: 0,
                    session_id: None,
                });
                // SPEC §6.3: a `not_found` profile is "this slot in
                // the chain has no key"; it counts as one failed
                // attempt, but the chain advances to the next entry
                // (if any). When this was the LAST entry, the
                // chain-exhausted bail at the end of the function
                // surfaces the failure with full meta.toml.
                continue;
            }
        };

        // Materialise readonly_mirror for this profile attempt. Hand
        // the sync filesystem work off to the blocking pool so a slow
        // copy doesn't stall the tokio runtime. SPEC §11: the entire
        // context surface comes from the prompt's frontmatter
        // `context_files` (no project-level `always_include`).
        let project_root_owned = project_root.to_path_buf();
        let context_files_owned = frontmatter.context_files.clone();
        let workspace = tokio::task::spawn_blocking(move || {
            isolation::create_readonly_mirror(&project_root_owned, &context_files_owned)
        })
        .await
        .context("isolation::create_readonly_mirror task panicked")??;

        // Re-check store_drained AFTER mirror creation — readonly_mirror
        // can take seconds for large `context_files` trees, during which
        // a peer alias's KeyDead handler may have flipped the flag.
        // Without this, we'd waste a cursor-agent spawn against an
        // already-empty store. (SPEC §6.3 store-drained guard.)
        if store_drained.load(Ordering::Relaxed) {
            bail_drain_with_meta!(format!(
                "aborted: {STORE_DRAINED_MSG} (peer alias drained the store while building the readonly mirror)"
            ));
        }

        // SPEC §8.5: KeyDead path advanced to a new profile. We skip
        // the cross-account resume probe (cursor backend silently
        // drops history; see §8.5.1) and go straight to a fresh
        // prompt with a continuation prefix that tells the new model
        // the previous session was interrupted. The carve-out below
        // skips the prefix when the previous chain step was a
        // `not_found` (profile didn't exist; cursor-agent was never
        // launched, so there is literally nothing to "continue").
        // KeyDead-without-session-id — cursor backend rejected the
        // key at HTTP layer before any stream-json init event, so
        // `last_session_id` is `None` even though a real attempt
        // happened — IS a real attempt; SPEC §8.5 says the next
        // profile must receive the continuation prefix even when
        // no session_id was captured.
        let prev_was_real_attempt = attempts
            .last()
            .map(|a| a.error_class.as_deref() != Some("not_found"))
            .unwrap_or(false);
        let mut state: CallState = if idx == 0 || !prev_was_real_attempt {
            CallState::Fresh
        } else {
            CallState::FreshWithContinuationPrefix
        };
        // SPEC §6.3: per-profile retry up to 3 times for Transient.
        const MAX_TRANSIENT_RETRIES: usize = 3;
        const BACKOFF_MS: [u64; 3] = [1000, 3000, 10000];
        let mut retry_count = 0usize;
        // Per-profile session_id captured from the most recent
        // cursor-agent call within this profile attempt. Used by
        // SPEC §8.4 transient retry to resume the same session.
        let mut profile_session_id: Option<String> = None;

        // Inner state-machine loop. Exits via `break (call_result, e_ms)`
        // when one of: (a) success, (b) classification done and not
        // a transient-retry case (KeyDead / ModelUnavailable / Unknown
        // / Transient retries exhausted).
        let (call_result, last_call_ms) = 'inner: loop {
            // SPEC §6.3: at the very top of every inner-loop iteration
            // (including the first), check the shared drain flag. The
            // outer chain-step check above can race with a peer alias
            // KeyDead-deleting the store between iterations.
            if store_drained.load(Ordering::Relaxed) {
                bail_drain_with_meta!(format!(
                    "aborted: {STORE_DRAINED_MSG} (peer alias drained the store mid-profile)"
                ));
            }
            // Build prompt body for the current state. Re-build every
            // iteration because state transitions (Fresh →
            // TransientResume, etc.) change the prompt.
            let body =
                build_prompt_for_state(&state, suppress_readonly, &user_prompt, &prompt_text);
            // SPEC §10.1: indirect-prompt threshold is evaluated on
            // the **current body**, not the original prompt — state
            // transitions can grow the body by ~200 bytes and push
            // a borderline prompt over the limit.
            let body_use_indirect = !dry_run
                && ((body.len() as u64) > inline_prompt_max_bytes
                    || (cfg!(windows) && body.bytes().any(|b| b > 127)));
            let cmdline_prompt = materialise_prompt(&workspace, &body, body_use_indirect)?;
            if body_use_indirect {
                crate::pln!(
                    "[{alias}] phase={} → wrote prompt ({} bytes) to .a2a/_prompt.md",
                    state.phase_tag(),
                    body.len()
                );
            }
            // SPEC §6.3: final drain check immediately before the
            // cursor-agent spawn. `materialise_prompt` (above) can do
            // a non-trivial filesystem write in indirect-prompt mode;
            // a peer alias's KeyDead handler can flip the drain flag
            // during that window, and SPEC §6.3 explicitly forbids
            // launching a fresh cursor-agent call against an already-
            // empty store. The drain check at the top of this loop
            // does not cover this window.
            if store_drained.load(Ordering::Relaxed) {
                bail_drain_with_meta!(format!(
                    "aborted: {STORE_DRAINED_MSG} (peer alias drained the store while preparing the prompt)"
                ));
            }

            let spec = CursorAgentSpec {
                cursor_model: model_cfg.cursor_model.clone(),
                mode: mode.to_string(),
                workspace: workspace.root().to_path_buf(),
                prompt_text: cmdline_prompt,
                api_key: profile.api_key.clone(),
                sandbox: sandbox.map(str::to_string),
                resume_session_id: state.resume_session_id(),
            };

            crate::pln!(
                "[{alias}] profile={profile_name} → calling cursor-agent (phase={}{})",
                state.phase_tag(),
                match state.resume_session_id() {
                    Some(ref s) => format!(", --resume {s}"),
                    None => String::new(),
                }
            );
            let started = Instant::now();
            let r = cursor_agent::run_with_progress(&spec, Some(alias)).await;
            let e_ms = started.elapsed().as_millis() as u64;

            // Capture session_id (SPEC §14.4).
            if let Ok(out) = &r
                && let Some(sid) = &out.session_id
                && !sid.is_empty()
            {
                profile_session_id = Some(sid.clone());
                last_session_id = Some(sid.clone());
                if !session_ids.contains(sid) {
                    session_ids.push(sid.clone());
                }
            }

            let is_failure = match &r {
                Ok(out) => out.exit_code != Some(0) || out.stream_is_error,
                Err(_) => true,
            };
            if !is_failure {
                break 'inner (r, e_ms);
            }

            // Failure path. Classify + route based on current state.
            let stderr_text = match &r {
                Ok(out) => out.stderr.clone(),
                Err(e) => format!("{e:#}"),
            };
            let class = classify(&stderr_text);

            // Transient + retry budget left → switch to TransientResume
            // state for the next iteration; sleep with cancel polling.
            if class == ErrorClass::Transient && retry_count < MAX_TRANSIENT_RETRIES {
                let delay_ms = BACKOFF_MS[retry_count.min(BACKOFF_MS.len() - 1)];
                let attempt_num = retry_count + 1;
                crate::pln!(
                    "[{alias}] profile={profile_name} → transient error; retrying after {delay_ms}ms (attempt {attempt_num}/{MAX_TRANSIENT_RETRIES})"
                );
                // Sleep in 200ms slices so a peer alias's KeyDead-drain
                // signal aborts us within ~200ms instead of the full
                // 1s/3s/10s backoff.
                let mut remaining = std::time::Duration::from_millis(delay_ms);
                let tick = std::time::Duration::from_millis(200);
                while !remaining.is_zero() {
                    let step = remaining.min(tick);
                    tokio::time::sleep(step).await;
                    if store_drained.load(Ordering::Relaxed) {
                        bail_drain_with_meta!(format!(
                            "aborted: {STORE_DRAINED_MSG} (peer alias signalled drain during transient backoff)"
                        ));
                    }
                    remaining -= step;
                }
                retry_count += 1;
                // Next call: resume the captured session if we have
                // one; otherwise (extremely early failure) re-send
                // fresh prompt — TransientResume requires a session.
                state = match profile_session_id.clone() {
                    Some(sid) => CallState::TransientResume(sid),
                    None => state, // keep current state; just retry
                };
                continue 'inner;
            }

            // All other terminal cases (KeyDead, ModelUnavailable,
            // Unknown, Transient with retries exhausted) → break out
            // and let outer handler classify + route.
            break 'inner (r, e_ms);
        };
        // ModelMeta.elapsed_ms is the cumulative wall time from
        // fallback start (covers all profile attempts in the chain).
        // FallbackAttempt.elapsed_ms below uses the per-call wall
        // time of the LAST cursor-agent invocation in this profile's
        // inner state-machine loop (transient retries collapse into
        // one row per SPEC §14.4 / §8.4).
        let elapsed_ms = started_total.elapsed().as_millis() as u64;

        // SPEC §3 / §7: detect concurrent `a2a clean --yes` mid-run.
        // If the readonly_mirror tempdir vanished while cursor-agent
        // was running, neither the answer it produced nor the
        // failure metadata we'd write makes sense — bail with a
        // clear error pointing at the likely cause.
        workspace.assert_alive()?;

        match call_result {
            Ok(out) if out.exit_code == Some(0) && !out.stream_is_error => {
                // Success.
                std::fs::write(&answer_path, &out.stdout)
                    .with_context(|| format!("write {}", answer_path.display()))?;
                if let Err(e) = db.record_last_used(profile_name) {
                    tracing::warn!(
                        "could not update last_used_at for profile '{profile_name}' \
                         (consultation succeeded; bookkeeping only): {e:#}"
                    );
                }
                // SPEC §14: when `--log-budget` is set, attach a
                // `BudgetInfo` (char-count breakdown) to this model's
                // `[[models]]` row in meta.toml. `always_chars` is
                // permanently 0 (SPEC §11 dropped `always_include`;
                // all context lives in the prompt's frontmatter).
                let budget = if log_budget {
                    let project_root_owned = project_root.to_path_buf();
                    let context_files_owned = frontmatter.context_files.clone();
                    let ctx_chars = tokio::task::spawn_blocking(move || {
                        context_files_owned
                            .iter()
                            .map(|p| crate::runner::count_chars_in_entry(&project_root_owned, p))
                            .sum::<usize>()
                    })
                    .await
                    .unwrap_or(0);
                    Some(BudgetInfo {
                        prompt_chars: original_prompt_chars,
                        context_chars: ctx_chars,
                        always_chars: 0,
                        answer_chars: out.stdout.chars().count(),
                    })
                } else {
                    None
                };
                crate::pln!(
                    "[{alias}] profile={profile_name} → OK ({:.1}s)",
                    elapsed_ms as f64 / 1000.0
                );
                attempts.push(FallbackAttempt {
                    profile: profile_name.clone(),
                    success: true,
                    error_class: None,
                    error_excerpt: None,
                    elapsed_ms: last_call_ms,
                    session_id: profile_session_id.clone(),
                });
                append_model_meta(
                    consult_dir,
                    topic,
                    ModelMeta {
                        alias: alias.to_string(),
                        cursor_model: model_cfg.cursor_model.clone(),
                        mode: mode.to_string(),
                        profile_used: profile_name.clone(),
                        fallback_chain: chain.clone(),
                        fallback_attempts: attempts.clone(),
                        success: true,
                        elapsed_ms,
                        answer_path: answer_path.clone(),
                        session_ids: session_ids.clone(),
                        last_session_id: last_session_id.clone(),
                        budget,
                    },
                )
                .await?;
                return Ok(());
            }
            // Failure path: classify and route.
            other => {
                let stderr_text = match &other {
                    Ok(out) => out.stderr.clone(),
                    Err(e) => format!("{e:#}"),
                };
                let class = classify(&stderr_text);
                let excerpt = stderr_excerpt(&stderr_text, 8);
                attempts.push(FallbackAttempt {
                    profile: profile_name.clone(),
                    success: false,
                    error_class: Some(format!("{class:?}").to_lowercase()),
                    error_excerpt: Some(excerpt.clone()),
                    elapsed_ms: last_call_ms,
                    session_id: profile_session_id.clone(),
                });
                match class {
                    ErrorClass::KeyDead => {
                        // SPEC §6.3: delete profile, then try next in chain.
                        crate::pln!(
                            "[{alias}] profile={profile_name} → KeyDead detected; deleting this profile from credentials.db. Original error:"
                        );
                        for line in excerpt.lines() {
                            crate::pln!("    {line}");
                        }
                        let became_empty = match auth::delete_profile_on_key_dead(
                            &mut db,
                            profile_name,
                        ) {
                            Ok(empty) => {
                                crate::pln!(
                                    "[{alias}] profile '{profile_name}' deleted; advancing fallback chain"
                                );
                                empty
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "could not delete dead profile '{profile_name}': {e:#}"
                                );
                                false
                            }
                        };
                        // Store drained: signal peer alias tasks to bail
                        // and short-circuit our own loop. Continuing
                        // would just spawn another cursor-agent call we
                        // already know cannot authenticate.
                        if became_empty {
                            store_drained.store(true, Ordering::Relaxed);
                            crate::pln!(
                                "[{alias}] credentials store is now empty; aborting this alias and signalling peers"
                            );
                            if let Err(e) = write_failure_meta(
                                consult_dir,
                                topic,
                                alias,
                                model_cfg,
                                mode,
                                &chain,
                                attempts.clone(),
                                started_total.elapsed().as_millis() as u64,
                                session_ids.clone(),
                                last_session_id.clone(),
                            )
                            .await
                            {
                                tracing::warn!(
                                    "could not write meta.toml failure record (the user-facing error below is the real cause): {e:#}"
                                );
                            }
                            bail!(
                                "aborted: {STORE_DRAINED_MSG} (last KeyDead deleted the only remaining profile '{profile_name}')"
                            );
                        }
                        if idx + 1 >= chain.len() {
                            if let Err(e) = write_failure_meta(
                                consult_dir,
                                topic,
                                alias,
                                model_cfg,
                                mode,
                                &chain,
                                attempts.clone(),
                                started_total.elapsed().as_millis() as u64,
                                session_ids.clone(),
                                last_session_id.clone(),
                            )
                            .await
                            {
                                tracing::warn!(
                                    "could not write meta.toml failure record (the user-facing error below is the real cause): {e:#}"
                                );
                            }
                            bail!(
                                "all profiles in chain failed for model alias '{alias}' (last: KeyDead)"
                            );
                        }
                        continue;
                    }
                    ErrorClass::ModelUnavailable | ErrorClass::Unknown => {
                        // SPEC §6.3: skip the alias entirely.
                        crate::pln!(
                            "[{alias}] profile={profile_name} → {} ({class:?}); skipping this model alias",
                            if class == ErrorClass::ModelUnavailable {
                                "model not available on this account"
                            } else {
                                "unrecognised error"
                            }
                        );
                        for line in excerpt.lines() {
                            crate::pln!("    {line}");
                        }
                        if let Err(e) = write_failure_meta(
                            consult_dir,
                            topic,
                            alias,
                            model_cfg,
                            mode,
                            &chain,
                            attempts.clone(),
                            started_total.elapsed().as_millis() as u64,
                            session_ids.clone(),
                            last_session_id.clone(),
                        )
                        .await
                        {
                            tracing::warn!(
                                "could not write meta.toml failure record (the user-facing error below is the real cause): {e:#}"
                            );
                        }
                        return Err(crate::user_err(format!(
                            "skipped: {class:?} on profile '{profile_name}'"
                        )));
                    }
                    ErrorClass::Transient => {
                        // Retries already exhausted; SPEC §6.3 says
                        // "transient → retry 3 → fail → skip alias".
                        crate::pln!(
                            "[{alias}] profile={profile_name} → Transient retry budget exhausted; skipping this model alias"
                        );
                        for line in excerpt.lines() {
                            crate::pln!("    {line}");
                        }
                        if let Err(e) = write_failure_meta(
                            consult_dir,
                            topic,
                            alias,
                            model_cfg,
                            mode,
                            &chain,
                            attempts.clone(),
                            started_total.elapsed().as_millis() as u64,
                            session_ids.clone(),
                            last_session_id.clone(),
                        )
                        .await
                        {
                            tracing::warn!(
                                "could not write meta.toml failure record (the user-facing error below is the real cause): {e:#}"
                            );
                        }
                        return Err(crate::user_err(format!(
                            "skipped: Transient retries exhausted on profile '{profile_name}'"
                        )));
                    }
                }
            }
        }
    }

    // Chain exhausted with no success.
    if let Err(e) = write_failure_meta(
        consult_dir,
        topic,
        alias,
        model_cfg,
        mode,
        &chain,
        attempts.clone(),
        started_total.elapsed().as_millis() as u64,
        session_ids.clone(),
        last_session_id.clone(),
    )
    .await
    {
        tracing::warn!(
            "could not write meta.toml failure record (the user-facing error below is the real cause): {e:#}"
        );
    }
    bail!("all profiles in chain failed for model alias '{alias}'")
}

#[allow(clippy::too_many_arguments)]
async fn write_failure_meta(
    consult_dir: &Path,
    topic: &str,
    alias: &str,
    model_cfg: &ModelAlias,
    mode: &str,
    chain: &[String],
    attempts: Vec<FallbackAttempt>,
    elapsed_ms: u64,
    session_ids: Vec<String>,
    last_session_id: Option<String>,
) -> Result<()> {
    // `profile_used` is the profile that was actually attempted last
    // (and presumably failed), not `chain.last()` which is just
    // "the declared chain's tail" — those differ whenever the run
    // aborted before reaching the chain end (ModelUnavailable,
    // Unknown, Transient retries exhausted on chain[0], etc).
    let profile_used = attempts
        .last()
        .map(|a| a.profile.clone())
        .unwrap_or_else(|| chain.last().cloned().unwrap_or_default());
    append_model_meta(
        consult_dir,
        topic,
        ModelMeta {
            alias: alias.to_string(),
            cursor_model: model_cfg.cursor_model.clone(),
            // Effective mode (CLI override or alias's stored
            // `default_mode`), not blindly `model_cfg.default_mode` —
            // otherwise `a2a ask --mode plan` would record
            // `mode = "agent"` on failure and mislead forensics.
            mode: mode.to_string(),
            profile_used,
            fallback_chain: chain.to_vec(),
            fallback_attempts: attempts,
            success: false,
            elapsed_ms,
            answer_path: PathBuf::new(),
            session_ids,
            last_session_id,
            budget: None,
        },
    )
    .await?;
    Ok(())
}

/// Take the last `n` non-empty lines of stderr for command-line display.
fn stderr_excerpt(stderr: &str, n: usize) -> String {
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    let take = lines.len().saturating_sub(n.min(lines.len()));
    lines[take..].join("\n")
}

#[cfg(test)]
mod classify_tests {
    use super::{ErrorClass, classify, contains_token};

    #[test]
    fn token_boundary_matches() {
        assert!(contains_token("status 401", "401"));
        assert!(contains_token("error: 401", "401"));
        assert!(contains_token("(401)", "401"));
        assert!(contains_token("[401]", "401"));
        assert!(contains_token("401\n", "401"));
        assert!(contains_token("\n401\n", "401"));
        assert!(contains_token("401", "401"));
        assert!(contains_token("HTTP/2 401", "401"));
    }

    #[test]
    fn token_boundary_rejects_substring() {
        assert!(!contains_token("4019", "401"));
        assert!(!contains_token("14012", "401"));
        assert!(!contains_token("a401b", "401"));
    }

    #[test]
    fn classifies_401_in_natural_phrasings() {
        assert_eq!(classify("Request failed: status 401"), ErrorClass::KeyDead);
        assert_eq!(classify("Error: 401\n"), ErrorClass::KeyDead);
        assert_eq!(classify("HTTP/2 401\n"), ErrorClass::KeyDead);
        assert_eq!(classify("(401)"), ErrorClass::KeyDead);
        assert_eq!(classify("401"), ErrorClass::KeyDead);
    }

    #[test]
    fn classifies_429_in_natural_phrasings() {
        assert_eq!(classify("status 429"), ErrorClass::Transient);
        assert_eq!(
            classify("HTTP 429 Too Many Requests"),
            ErrorClass::Transient
        );
    }

    #[test]
    fn does_not_misclassify_substring_numbers() {
        // 4019 / 1429 should not trigger 401 / 429.
        assert_eq!(classify("error code 4019"), ErrorClass::Unknown);
        assert_eq!(classify("port 14290"), ErrorClass::Unknown);
    }

    #[test]
    fn classifies_billing_keywords() {
        assert_eq!(classify("Payment overdue"), ErrorClass::KeyDead);
        assert_eq!(
            classify("quota exceeded for this month"),
            ErrorClass::KeyDead
        );
    }

    #[test]
    fn classifies_model_unavailable() {
        assert_eq!(
            classify("Cannot use this model: gpt-5.5-extra-high"),
            ErrorClass::ModelUnavailable
        );
    }

    #[test]
    fn classifies_unknown() {
        assert_eq!(classify("something weird happened"), ErrorClass::Unknown);
        assert_eq!(classify(""), ErrorClass::Unknown);
    }
}
