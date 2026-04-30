// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! Wrap a `cursor-agent` invocation as an async subprocess.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

/// RAII guard that aborts a tokio task when dropped.
///
/// `tokio::task::JoinHandle::Drop` does NOT abort the underlying task
/// (this is documented tokio behaviour). On the hard-timeout path the
/// outer `body` future is dropped, dropping the JoinHandle, but the
/// progress task continues to print `[<alias>] still alive ...` lines
/// until the runtime itself shuts down — which during multi-model
/// parallel runs is well after the timed-out alias should be silent.
/// Wrapping the handle in this guard ensures `abort()` fires whenever
/// the guard's owner (here: `body`) is dropped.
struct AbortOnDrop(Option<tokio::task::JoinHandle<()>>);

impl AbortOnDrop {
    fn new(h: tokio::task::JoinHandle<()>) -> Self {
        Self(Some(h))
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            h.abort();
        }
    }
}

/// Windows: prevent spawned subprocess (powershell / node) from popping
/// up a new console window when a2a is invoked from a non-TTY parent
/// (e.g. an IDE chat shell). This is the
/// `CREATE_NO_WINDOW` process-creation flag from the Win32 API.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Apply `CREATE_NO_WINDOW` on Windows; no-op everywhere else.
fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[derive(Debug, Clone)]
pub struct CursorAgentSpec {
    /// Cursor CLI's `--model <name>` value.
    pub cursor_model: String,
    /// `agent` (default, allows writes) or `plan` (read-only).
    /// SPEC §8.0: clap-validated to one of these two values.
    pub mode: String,
    /// Working directory passed via `--workspace`.
    pub workspace: PathBuf,
    /// Prompt body passed as a positional arg (or an indirect-prompt
    /// redirect, when `prompt_text.len()` > inline cap).
    pub prompt_text: String,
    /// API key for `CURSOR_API_KEY` env var.
    pub api_key: String,
    /// SPEC §8.0: `--sandbox <enabled|disabled>` passthrough. `None`
    /// means don't pass the flag (cursor-agent uses sandbox.json).
    pub sandbox: Option<String>,
    /// SPEC §8.4 only: when set, append `--resume <session_id>` so
    /// cursor backend continues the same chat. Used **exclusively**
    /// for transient retries within a single profile (same account).
    /// SPEC §8.5.1 explicitly forbids cross-account resume — the
    /// cursor backend silently drops history when the api_key
    /// belongs to a different account than the chat owner. On
    /// KeyDead profile-switch this field MUST be `None`; the
    /// continuation hint is delivered through
    /// `CallState::FreshWithContinuationPrefix`'s prompt body.
    pub resume_session_id: Option<String>,
}

pub struct CursorAgentResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    /// `result` event reported `is_error: true`. cursor-agent can exit
    /// 0 in this case (task failed, not binary), so callers must
    /// check this in addition to exit_code.
    pub stream_is_error: bool,
    /// `subtype` field from the stream-json `result` event when
    /// `is_error == true`.
    pub stream_subtype: Option<String>,
    /// SPEC §14.4: `session_id` (chatId) parsed from the stream-json
    /// `type=system, subtype=init` event (or any later event that
    /// carries it, as a fallback). `None` means the stream never
    /// reached the init line — usually a hard early failure
    /// (network refused / binary missing / invalid key).
    pub session_id: Option<String>,
}

pub fn locate_binary() -> Option<PathBuf> {
    // On Windows the cursor-agent ships as `cursor-agent.cmd` (a thin
    // forwarder to the powershell script `cursor-agent.ps1`).  Calling
    // the `.cmd` directly forces our prompt text through cmd.exe's
    // quoting rules, which mangles any prompt containing newlines or
    // double quotes.  We therefore prefer `.ps1` so we can spawn it via
    // `powershell.exe -File` (one-layer quoting handled by PowerShell).
    //
    // Fall back to the `.cmd` shim only if `.ps1` is not where we expect.
    let names: &[&str] = if cfg!(windows) {
        &[
            "cursor-agent.ps1",
            "agent.ps1",
            "cursor-agent.exe",
            "cursor-agent.cmd",
            "agent.exe",
            "agent.cmd",
        ]
    } else {
        &["cursor-agent", "agent"]
    };
    for n in names {
        if let Ok(path) = which::which(n) {
            return Some(path);
        }
    }
    // PowerShell scripts may not be in PATH via `which` because Windows
    // PATH probing is .exe-centric.  Try the canonical install dir as a
    // last resort.
    if cfg!(windows) {
        let candidates = [
            std::env::var("LOCALAPPDATA").ok().map(|p| {
                PathBuf::from(p)
                    .join("cursor-agent")
                    .join("cursor-agent.ps1")
            }),
            std::env::var("LOCALAPPDATA")
                .ok()
                .map(|p| PathBuf::from(p).join("cursor-agent").join("agent.ps1")),
        ];
        for cand in candidates.into_iter().flatten() {
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Run cursor-agent and emit progress to stdout: a `[<alias>] received first
/// response` line at the first assistant token, plus periodic char-count
/// updates every 10 s when the count has changed since the previous tick.
pub async fn run_with_progress(
    spec: &CursorAgentSpec,
    alias_for_progress: Option<&str>,
) -> Result<CursorAgentResult> {
    let bin = locate_binary().ok_or_else(|| {
        anyhow::anyhow!("cursor-agent not found in PATH (run `a2a doctor` for install help)")
    })?;

    let is_ps1 = bin.extension().and_then(|s| s.to_str()) == Some("ps1");

    // Windows command line is limited to 32 768 characters per
    // `CreateProcessW`; a few thousand chars are eaten by the binary
    // path and the other flags. As of r10 the fallback layer
    // automatically switches to indirect-prompt mode (writes to
    // `<workspace>/.a2a/_prompt.md`, passes a short redirect on the
    // cmdline) once the prompt exceeds `[defaults]
    // inline_prompt_max_bytes` (default 24 000). This hard 28 000-byte
    // cap below is therefore a *backstop*: it only triggers if the
    // user has explicitly raised `inline_prompt_max_bytes` above
    // 28 000 (and, on Windows, that's a configuration mistake — the
    // OS will refuse the spawn). Friendly bail beats Windows
    // `ERROR_FILENAME_EXCED_RANGE`.
    #[cfg(windows)]
    {
        const WIN_PROMPT_SOFT_CAP: usize = 28_000;
        if spec.prompt_text.len() > WIN_PROMPT_SOFT_CAP {
            anyhow::bail!(
                "prompt is {} bytes; Windows command-line cap prevents safely spawning \
                cursor-agent above {} bytes. a2a normally auto-switches to indirect-prompt \
                mode for large prompts — your `[defaults] inline_prompt_max_bytes` is set \
                too high. Lower it (default 24000) or leave it unset.",
                spec.prompt_text.len(),
                WIN_PROMPT_SOFT_CAP
            );
        }
    }

    // Build the command. The Windows .ps1 path requires special handling:
    // `powershell -File <ps1> <args...>` re-tokenises args on whitespace
    // (including newlines), which mangles any prompt body with line breaks.
    // Instead use `-Command` mode and pass the entire invocation as a
    // single PowerShell expression with single-quoted string literals
    // (PowerShell single-quote rules: every char is literal except `'`,
    // which is escaped as `''`). This keeps prompt newlines intact.
    //
    // Output mode: `stream-json` (NDJSON) gives us per-event progress so
    // we can show "first response received" + 10-second char-count
    // updates. We aggregate `assistant` event bodies into the final
    // text so the rest of the pipeline still sees a plain answer string.
    let mut cmd = if is_ps1 {
        let mut c = Command::new("powershell.exe");
        c.arg("-NoProfile");
        c.arg("-ExecutionPolicy").arg("Bypass");
        c.arg("-Command");

        let esc = |s: &str| s.replace('\'', "''");
        let ps1_q = esc(&bin.to_string_lossy());
        let model_q = esc(&spec.cursor_model);
        let ws_q = esc(&spec.workspace.to_string_lossy());
        let prompt_q = esc(&spec.prompt_text);

        // cursor-agent's `--mode` only accepts `plan` and `ask`.
        // `agent` is the default when the flag is omitted.  Don't pass
        // `--mode agent` because cursor-agent will reject it.
        let mode_part = if spec.mode == "agent" || spec.mode.is_empty() {
            String::new()
        } else {
            let mode_q = esc(&spec.mode);
            format!(" '--mode' '{mode_q}'")
        };
        let mut script = format!(
            "& '{ps1_q}' '-p' '--output-format' 'stream-json' '--stream-partial-output' '--model' '{model_q}'{mode_part} '--workspace' '{ws_q}' '--trust'"
        );
        if let Some(sb) = &spec.sandbox {
            let sb_q = esc(sb);
            script.push_str(&format!(" '--sandbox' '{sb_q}'"));
        }
        if let Some(sid) = &spec.resume_session_id {
            let sid_q = esc(sid);
            script.push_str(&format!(" '--resume' '{sid_q}'"));
        }
        // `--` terminates option parsing; needed because the prompt body
        // can begin with `---` (YAML frontmatter), which cursor-agent's
        // CLI parser would otherwise treat as a long flag.
        script.push_str(&format!(" '--' '{prompt_q}'; exit $LASTEXITCODE"));
        c.arg(script);
        c
    } else {
        let mut c = Command::new(&bin);
        c.arg("-p");
        c.arg("--output-format").arg("stream-json");
        c.arg("--stream-partial-output");
        c.arg("--model").arg(&spec.cursor_model);
        // cursor-agent: `agent` is implicit (no flag); `--mode` only
        // accepts `plan` / `ask`.
        if spec.mode != "agent" && !spec.mode.is_empty() {
            c.arg("--mode").arg(&spec.mode);
        }
        c.arg("--workspace").arg(&spec.workspace);
        c.arg("--trust");
        if let Some(sb) = &spec.sandbox {
            c.arg("--sandbox").arg(sb);
        }
        if let Some(sid) = &spec.resume_session_id {
            c.arg("--resume").arg(sid);
        }
        c.arg("--"); // option-parsing terminator (see comment above)
        c.arg(&spec.prompt_text);
        c
    };

    cmd.env("CURSOR_API_KEY", &spec.api_key);
    // Strip CURSOR_API_KEY_FILE so a parent-side env var pointing at
    // a credential file isn't leaked to the cursor-agent subprocess
    // (cursor-agent would otherwise prefer the file over our env).
    cmd.env_remove("CURSOR_API_KEY_FILE");

    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    no_window(&mut cmd);
    // If a2a itself is dropped (panic / Ctrl+C / process exit) take the
    // child cursor-agent down with it. Without this the child can keep
    // running, burning the user's API quota long after the parent died.
    cmd.kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", bin.display()))?;

    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    // Shared progress state — char count seen so far in the assistant
    // text stream, plus a flag for "first response printed".
    let chars_seen = Arc::new(AtomicUsize::new(0));
    let first_response_printed = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    // Shared session_id slot. The stdout aggregator writes here as
    // soon as it parses a `session_id` field from any stream-json
    // event. The hard-timeout path reads here so a transient retry
    // (SPEC §8.4) can still `--resume` even when cursor-agent stalls
    // for >15 minutes — the local `session_id` variable inside the
    // aggregator future is dropped on timeout cancellation, but this
    // shared slot survives.
    let captured_session_id = Arc::new(std::sync::Mutex::new(None::<String>));
    // SPEC §6.2 / §13: the timeout path used to discard whatever
    // stderr cursor-agent had emitted before the stall, replacing it
    // with a synthetic one-line `cursor-agent timeout: ...` message.
    // That broke two things: (a) §13's "last 8 lines of stderr"
    // diagnostic was useless, and (b) classification could mis-route
    // a real KeyDead (`unauthorized` / `401` already in the buffer)
    // as Transient because the synthetic line only contains the
    // word `timeout`. Mirror chunks read by the stderr aggregator
    // into a shared slot so the timeout path can recover them
    // exactly the way `captured_session_id` survives cancellation.
    let captured_stderr = Arc::new(std::sync::Mutex::new(String::new()));

    // Spawn the periodic progress task (every 10 s).  Only emits when
    // the char count has changed since the previous tick. Wrapped in
    // `AbortOnDrop` so the timeout-cancellation path (which drops
    // `body` and therefore the handle) actually stops the task —
    // tokio's `JoinHandle::Drop` is a no-op by itself.
    let progress_task = AbortOnDrop::new({
        let chars_seen = chars_seen.clone();
        let done = done.clone();
        let alias = alias_for_progress.map(str::to_string);
        tokio::spawn(async move {
            let mut last = 0usize;
            // Wall-clock timestamp of the most recent observable
            // change in stream state (either streamed-text growth or
            // an "alive" tick we already emitted). Used to throttle
            // the "still alive" line so it does not spam during long
            // reasoning / tool-call phases.
            let mut last_progress_at = std::time::Instant::now();
            // How long we tolerate silence before emitting the first
            // "alive" notice. Then re-throttle to the same interval
            // so subsequent quiet periods still surface roughly
            // every 30 seconds — enough to reassure the user without
            // turning into a per-tick log spam.
            const QUIET_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(30);
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                if done.load(Ordering::Relaxed) {
                    break;
                }
                let now = chars_seen.load(Ordering::Relaxed);
                if now != last {
                    let delta = now.saturating_sub(last);
                    if let Some(a) = &alias {
                        crate::pln!(
                            "[{a}] still receiving... +{delta} chars (total {now} chars in last 10s)"
                        );
                    } else {
                        crate::pln!("still receiving... +{delta} chars (total {now} chars)");
                    }
                    last = now;
                    last_progress_at = std::time::Instant::now();
                } else {
                    // No streamed-text growth since the previous
                    // tick.  cursor-agent could legitimately be busy
                    // doing things that don't manifest as
                    // assistant-delta events: model "thinking" /
                    // reasoning events, internal tool calls (file
                    // read, grep, edit), or the final non-delta
                    // result aggregation. Without a visible signal
                    // here the user perceives a hang. Emit a single
                    // "alive" line every QUIET_THRESHOLD, then reset
                    // the timer so the line does not fire again at
                    // the next 10-second tick.
                    let quiet_for = last_progress_at.elapsed();
                    if quiet_for >= QUIET_THRESHOLD {
                        let secs = quiet_for.as_secs();
                        if let Some(a) = &alias {
                            crate::pln!(
                                "[{a}] still alive (no new streamed text in {secs}s; cursor-agent likely thinking / tool-calling)"
                            );
                        } else {
                            crate::pln!(
                                "still alive (no new streamed text in {secs}s; cursor-agent likely thinking / tool-calling)"
                            );
                        }
                        last_progress_at = std::time::Instant::now();
                    }
                }
            }
        })
    });

    // Reader: parse NDJSON line-by-line, accumulate assistant text into
    // a plain-text buffer (so the rest of the pipeline keeps the same
    // String contract), update progress counters, and print the
    // "first response received" line on the first assistant token.
    let stdout_aggregator = {
        let chars_seen = chars_seen.clone();
        let first_response_printed = first_response_printed.clone();
        let alias = alias_for_progress.map(str::to_string);
        let captured_session_id = captured_session_id.clone();
        async move {
            let mut text = String::new();
            let mut non_delta_text = String::new();
            let mut last_result_text: Option<String> = None;
            let mut stream_is_error = false;
            let mut stream_subtype: Option<String> = None;
            // SPEC §14.4: cursor-agent prints session_id on every
            // stream-json event (system / user / assistant / result).
            // Capture the first one we see — typically the
            // `type=system, subtype=init` line — so the caller can
            // record it in meta.toml for resume / audit.
            let mut session_id: Option<String> = None;
            if let Some(h) = stdout_handle {
                let mut reader = BufReader::new(h).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let v: serde_json::Value = match serde_json::from_str(line) {
                        Ok(v) => v,
                        Err(_) => {
                            // Non-JSON line — keep as-is in case cursor-agent
                            // emitted a free-text error before stream-json
                            // boot. Add to text so the caller can see it.
                            text.push_str(line);
                            text.push('\n');
                            continue;
                        }
                    };
                    if session_id.is_none()
                        && let Some(sid) = v.get("session_id").and_then(|x| x.as_str())
                        && !sid.is_empty()
                    {
                        session_id = Some(sid.to_string());
                        // Mirror to the shared slot so the timeout
                        // path can recover it after the aggregator
                        // future is dropped.
                        if let Ok(mut g) = captured_session_id.lock() {
                            *g = Some(sid.to_string());
                        }
                    }
                    let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
                    match typ {
                        "assistant" => {
                            // Aggregate all `content[].text` substrings.
                            // Some events are streaming deltas, some are
                            // buffered flushes; we count deltas only (the
                            // ones that have `timestamp_ms` present and
                            // `model_call_id` absent — see Cursor docs)
                            // to avoid double-counting the same text.
                            let is_delta =
                                v.get("timestamp_ms").is_some() && v.get("model_call_id").is_none();
                            let content = v
                                .get("message")
                                .and_then(|m| m.get("content"))
                                .and_then(|c| c.as_array());
                            if let Some(arr) = content {
                                for c in arr {
                                    if let Some(t) = c.get("text").and_then(|x| x.as_str()) {
                                        if is_delta {
                                            text.push_str(t);
                                            chars_seen
                                                .fetch_add(t.chars().count(), Ordering::Relaxed);
                                            if !first_response_printed.swap(true, Ordering::Relaxed)
                                            {
                                                if let Some(a) = &alias {
                                                    crate::pln!(
                                                        "[{a}] received first response (streaming...)"
                                                    );
                                                } else {
                                                    crate::pln!(
                                                        "received first response (streaming...)"
                                                    );
                                                }
                                            }
                                        } else {
                                            // Non-delta assistant text:
                                            // keep as a fallback in case
                                            // cursor-agent's stream
                                            // schema changes and our
                                            // delta heuristic stops
                                            // matching.
                                            non_delta_text.push_str(t);
                                        }
                                    }
                                }
                            }
                        }
                        "result" => {
                            // Terminal `result` event: capture canonical
                            // final text plus error metadata.
                            if let Some(t) = v.get("result").and_then(|x| x.as_str()) {
                                last_result_text = Some(t.to_string());
                            }
                            if v.get("is_error").and_then(|x| x.as_bool()) == Some(true) {
                                stream_is_error = true;
                                stream_subtype = v
                                    .get("subtype")
                                    .and_then(|x| x.as_str())
                                    .map(str::to_string);
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Prefer canonical result; otherwise delta accumulation;
            // otherwise non-delta text as last-resort fallback.
            let body = last_result_text
                .or(if !text.is_empty() { Some(text) } else { None })
                .unwrap_or(non_delta_text);
            (body, stream_is_error, stream_subtype, session_id)
        }
    };

    let stderr_aggregator = {
        let captured_stderr = captured_stderr.clone();
        async move {
            let mut buf = String::new();
            if let Some(mut h) = stderr_handle {
                // Chunked read so each chunk can be mirrored into the
                // shared `captured_stderr` slot; `read_to_string` would
                // hold everything in a future-local buffer that gets
                // dropped on timeout cancellation. UTF-8 multi-byte
                // splits across chunks are tolerated via
                // `from_utf8_lossy` — stderr keyword matching is ASCII
                // only (SPEC §6.2) and the user-facing 8-line summary
                // is best-effort prose, so a stray replacement char on
                // a torn boundary is harmless.
                let mut chunk = [0u8; 4096];
                loop {
                    match h.read(&mut chunk).await {
                        Ok(0) => break,
                        Ok(n) => {
                            let s = String::from_utf8_lossy(&chunk[..n]);
                            buf.push_str(&s);
                            if let Ok(mut g) = captured_stderr.lock() {
                                g.push_str(&s);
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
            buf
        }
    };

    // The hard timeout wraps the **entire** drain + wait body, not
    // just `child.wait()`. If cursor-agent stops emitting events
    // while keeping its pipes open (TLS half-open, wedged HTTP/2
    // stream, beta-build deadlocks), the aggregators' `read_to_string`
    // / `lines().next_line()` block forever waiting for EOF — a
    // `child.wait()`-only timeout never gets a chance to fire.
    //
    // 15 minutes is generous for a single cursor-agent invocation
    // (longest observed legitimate runs are ~6 minutes on heavy
    // reasoning models with tool calls). On timeout, dropping the
    // future moves `child` into `Drop` — `kill_on_drop(true)` (set
    // above) makes the OS reap cursor-agent automatically; the
    // aggregators unblock via cancellation when the body is dropped.
    const HARD_WAIT_CAP: std::time::Duration = std::time::Duration::from_secs(15 * 60);
    // Move `progress_task` (the `AbortOnDrop` guard) into `body` so
    // the timeout-cancellation path drops it together with the rest
    // of the body's locals — `Drop for AbortOnDrop` then aborts the
    // task. The success path also drops it on the way out.
    let body = async move {
        let ((stdout_text, stream_is_error, stream_subtype, session_id), stderr_bytes) =
            tokio::join!(stdout_aggregator, stderr_aggregator);
        done.store(true, Ordering::Relaxed);
        // progress_task auto-aborts when this scope ends (Drop guard);
        // explicit abort here is redundant.
        drop(progress_task);
        let status = child.wait().await.context("wait for cursor-agent")?;
        Ok::<_, anyhow::Error>(CursorAgentResult {
            stdout: stdout_text,
            stderr: stderr_bytes,
            exit_code: status.code(),
            stream_is_error,
            stream_subtype,
            session_id,
        })
    };
    match tokio::time::timeout(HARD_WAIT_CAP, body).await {
        Ok(r) => r,
        Err(_) => {
            // Recover any session_id parsed before the stall. We
            // cannot read the local `session_id` from the aggregator
            // future (it was dropped on cancellation), but the shared
            // `Mutex` slot survives. SPEC §8.4: this lets the
            // fallback runner do `--resume <captured_session_id>`
            // on the transient retry instead of replaying the full
            // prompt from scratch.
            let captured_sid = captured_session_id.lock().ok().and_then(|g| g.clone());
            // Recover whatever stderr cursor-agent emitted before the
            // stall. SPEC §6.2 / §13: keep the original lines so
            // (a) classification still sees keywords like
            // `unauthorized` / `401` (KeyDead trumps the synthetic
            // `timeout` keyword that maps to Transient — `classify`
            // checks KeyDead before Transient), and (b) §13's
            // "last 8 lines of stderr" diagnostic stays meaningful.
            let captured_se = captured_stderr
                .lock()
                .ok()
                .map(|g| g.clone())
                .unwrap_or_default();
            let synthetic = format!(
                "cursor-agent timeout: run exceeded {} seconds; killed via kill_on_drop",
                HARD_WAIT_CAP.as_secs()
            );
            let stderr = if captured_se.trim().is_empty() {
                synthetic
            } else {
                format!("{captured_se}\n{synthetic}")
            };
            // Return a soft failure (not bail!) so the fallback layer
            // classifies on stderr keywords ("timeout" → Transient
            // per SPEC §6.2 unless KeyDead/ModelUnavailable wins
            // earlier on a recovered keyword) and runs SPEC §8.4
            // transient retry with the recovered session_id.
            Ok(CursorAgentResult {
                stdout: String::new(),
                stderr,
                exit_code: None,
                stream_is_error: true,
                stream_subtype: Some("timeout".to_string()),
                session_id: captured_sid,
            })
        }
    }
}

pub fn dry_run_command_string(spec: &CursorAgentSpec) -> String {
    let sandbox = match &spec.sandbox {
        Some(v) => format!(" --sandbox {v}"),
        None => String::new(),
    };
    // Mirror the real spawn paths: cursor-agent's `--mode agent` is
    // rejected because `agent` is the implicit default. Only emit
    // the flag when mode is not the implicit default.
    let mode_part = if spec.mode == "agent" || spec.mode.is_empty() {
        String::new()
    } else {
        format!(" --mode {}", spec.mode)
    };
    format!(
        "CURSOR_API_KEY=$KEY cursor-agent -p --output-format stream-json --stream-partial-output \
         --model {}{mode_part} --workspace {} --trust{} -- <prompt>",
        spec.cursor_model,
        spec.workspace.display(),
        sandbox,
    )
}

/// Build a `Command` for invoking `bin`. Centralises the Windows
/// `.ps1 → powershell.exe -File` redirection so every cursor-agent
/// spawn point uses the same quoting strategy.
fn build_command(bin: &Path) -> Command {
    let is_ps1 = bin.extension().and_then(|s| s.to_str()) == Some("ps1");
    let mut c = if is_ps1 {
        let mut c = Command::new("powershell.exe");
        c.arg("-NoProfile");
        c.arg("-ExecutionPolicy").arg("Bypass");
        c.arg("-File").arg(bin);
        c
    } else {
        Command::new(bin)
    };
    no_window(&mut c);
    c
}

/// How long to wait for `cursor-agent --version` / `cursor-agent
/// status` / `cursor-agent --list-models` before giving up. These are
/// fast metadata calls; if they hang past 30 s something is wrong
/// (revoked key, network outage, hung child) and we want a clear
/// error rather than blocking the user's terminal forever.
const SUBPROC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub async fn run_version_check() -> Result<String> {
    let bin = locate_binary().ok_or_else(|| anyhow::anyhow!("cursor-agent not in PATH"))?;
    let mut cmd = build_command(&bin);
    cmd.env_remove("CURSOR_API_KEY_FILE")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Ensure the child dies if the awaiting task is dropped (user
        // Ctrl+Cs `a2a doctor`, or a sibling task panics).
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", bin.display()))?;
    let output = match tokio::time::timeout(SUBPROC_TIMEOUT, child.wait_with_output()).await {
        Ok(r) => r?,
        Err(_) => bail!("cursor-agent --version timed out after {SUBPROC_TIMEOUT:?}"),
    };
    if !output.status.success() {
        bail!("cursor-agent --version exited {}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn run_status_check(api_key: Option<&str>) -> Result<String> {
    let bin = locate_binary().ok_or_else(|| anyhow::anyhow!("cursor-agent not in PATH"))?;
    let mut cmd = build_command(&bin);
    cmd.arg("status");
    cmd.env_remove("CURSOR_API_KEY_FILE");
    if let Some(k) = api_key {
        cmd.env("CURSOR_API_KEY", k);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", bin.display()))?;
    let output = match tokio::time::timeout(SUBPROC_TIMEOUT, child.wait_with_output()).await {
        Ok(r) => r?,
        Err(_) => bail!("cursor-agent status timed out after {SUBPROC_TIMEOUT:?}"),
    };
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn read_prompt_text(prompt_file: &Path) -> Result<String> {
    std::fs::read_to_string(prompt_file).with_context(|| format!("read {}", prompt_file.display()))
}
