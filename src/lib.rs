// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! a2a — multi-AI consultation bridge.
//!
//! Library root. The CLI binary in `src/main.rs` calls [`run`] which parses
//! arguments and dispatches to the corresponding subcommand.

pub mod auth;
pub mod commands;
pub mod embedded;
pub mod fallback;
pub mod isolation;
pub mod paths;
pub mod prompt;
pub mod runner;
pub mod util;

use anyhow::Result;
use clap::Parser;
use thiserror::Error;

pub const A2A_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Hardcoded runtime defaults — SPEC §11. Every value here is a
/// build-time constant baked into the binary; changing any of them
/// requires a rebuild. There is no override flag or env var.
pub mod defaults {
    /// Run multiple model aliases concurrently. SPEC §11.
    pub const PARALLEL: bool = true;

    /// Output directory (project-relative) where every consultation
    /// run drops its `<ts>-<topic>-<uuid>/` subdir. SPEC §15.
    pub const OUTPUT_ROOT: &str = "consultations";

    /// Seconds between successive model spawns when `PARALLEL` is
    /// `true`. Avoids smashing the cursor backend with N parallel
    /// auth handshakes. SPEC §11.
    pub const STAGGER_SECS: u64 = 3;

    /// Switch to indirect-prompt mode (write to `.a2a/_prompt.md`,
    /// pass redirect on cmdline) once the prompt body exceeds this
    /// many bytes — SPEC §10.1. Conservative; well under the
    /// Windows 32K command-line cap.
    pub const INLINE_PROMPT_MAX_BYTES: u64 = 24_000;
}

/// A user-facing soft error: an expected outcome the user can correct,
/// such as "API key already exists in another profile" or "profile name
/// taken". The CLI binary treats this as a non-failure: prints the
/// message to stdout and exits with code 0, so shells do not show a
/// generic "command failed" stack trace.
///
/// System errors (DB corruption, missing toolchain, network failure)
/// remain plain `anyhow::Error` and exit with code 1 via stderr.
#[derive(Debug, Error)]
#[error("{0}")]
pub struct UserError(pub String);

/// "Business failure" — the operation reached cursor-agent / the
/// upstream model and the model returned a failure (network outage,
/// API quota, model not available on this account, etc.). The
/// per-model failure messages were already printed via `pln!`, so the
/// outer summary should NOT be wrapped as a system error: stack-trace
/// noise from PowerShell's `NativeCommandError` only confuses the
/// user. We still want a non-zero exit code for CI / scripts so they
/// can branch on success/failure.
///
/// Routing in `main.rs`:
///   - `Ok(())`              → exit 0
///   - `UserError`           → stdout, exit 0   (recoverable / expected)
///   - `BusinessFailure`     → stdout, exit 1   (upstream / external; not a2a's fault)
///   - other `anyhow::Error` → stderr, exit 1   (a2a system / programming error)
///
/// `BusinessFailure` exists so "all model tasks failed due to TLS /
/// quota / model-unavailable" doesn't trigger PowerShell's
/// `NativeCommandError` wrap (which would render a fake stack trace
/// pointing into its own temporary script wrapper).
#[derive(Debug, Error)]
#[error("{0}")]
pub struct BusinessFailure(pub String);

/// Construct a soft user-error wrapped in `anyhow::Error` so it flows
/// through the existing `Result<()>` plumbing but remains downcastable
/// in `main`.
#[macro_export]
macro_rules! user_bail {
    ($($t:tt)*) => {
        return ::std::result::Result::Err(::anyhow::Error::new(
            $crate::UserError(format!($($t)*))
        ))
    };
}

/// Helper to build a soft user-error from a String at runtime
/// (without `return`, unlike `user_bail!`).
pub fn user_err(msg: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(UserError(msg.into()))
}

/// Construct a `BusinessFailure` (operation reached upstream and the
/// upstream rejected — non-zero exit but no stack trace).
pub fn business_failure(msg: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(BusinessFailure(msg.into()))
}

/// `println!` followed by an immediate stdout flush.
///
/// Why: when a2a is launched from a non-TTY parent (Cursor IDE chat
/// shell, CI, scripts piping stdout), Rust's `println!` is block-
/// buffered, not line-buffered. Per-model progress lines (which is
/// the whole point of streaming feedback) accumulate inside the
/// process and arrive in a burst at the end. Flushing after every
/// progress line restores line-by-line UX in those environments.
#[macro_export]
macro_rules! pln {
    ($($t:tt)*) => {{
        use ::std::io::Write;
        println!($($t)*);
        let _ = ::std::io::stdout().flush();
    }};
}

/// Top-level CLI parser. See `commands::Cli` for subcommands.
pub fn run() -> Result<()> {
    init_tracing();
    let cli = commands::Cli::parse();
    if let Some(ref cmd) = cli.command {
        warn_if_cursor_agent_missing(cmd);
    }
    commands::dispatch(cli)
}

/// Stderr nudge when `cursor-agent` is not on PATH. Fires on every
/// subcommand invocation **except**:
///   - `a2a doctor` / `a2a status` — they have their own dedicated
///     cursor-agent reachability line; a stderr warning would just
///     duplicate that message.
///   - `a2a` (no subcommand → welcome wizard) — same reason: the
///     wizard prints a structured `[2/3] Cursor CLI check` block.
///     That branch never reaches this function because the caller
///     guards on `cli.command.is_some()`.
///
/// Rationale: the first command a fresh user runs is usually
/// `a2a auth add` or `a2a models add` (neither needs cursor-agent
/// itself), so a hard "not found" failure on a later `a2a ask` would
/// be the user's first signal that Cursor CLI was missing the whole
/// time. One stderr line per command is cheap insurance against that
/// surprise; the `which` probe takes < 5 ms in practice.
fn warn_if_cursor_agent_missing(cmd: &commands::Command) {
    if matches!(
        cmd,
        commands::Command::Doctor(_) | commands::Command::Status(_)
    ) {
        return;
    }
    if crate::runner::cursor_agent::locate_binary().is_some() {
        return;
    }
    eprintln!("[!!] cursor-agent not on PATH; `a2a ask` and `a2a models available` will fail.");
    eprintln!(
        "     Install Cursor CLI from https://cursor.com/cli, or run `a2a doctor` for platform-specific install instructions."
    );
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_env("A2A_LOG").unwrap_or_else(|_| EnvFilter::new("a2a=info"));
    let _ = fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
