# Changelog

All notable changes to this project are documented here.

## v0.1.0 — 2026-04-30 (initial release)

### CLI

- Subcommands: `init`, `ask`, `auth`, `doctor`, `list`, `clean`,
  `status`, `models`, `reset`.
- No-subcommand entry points:
  - `a2a` (typed in a terminal or double-clicked from Explorer) →
    welcome wizard. Detects whether the binary's directory is on
    user PATH, optionally appends it (HKCU\Environment\Path on
    Windows, manual recipe printed on Unix), and detects the
    `cursor-agent` CLI. Stale a2a directories on PATH (other
    locations whose `a2a.exe` differs from the running binary) are
    flagged and removable in one Y/n.
  - `a2a --agent` → structured health report (`[health]` block of
    `key: value` lines an AI agent can regex-parse) plus an
    `[next-step for you, the agent]` block worded as direct
    imperatives. Never pauses, never prompts for Y/n — safe to
    capture stdout in a single read from Cursor's terminal tool.
- Non-interactive credential entry via `--from-stdin` on
  `a2a auth add` / `a2a auth update`: the API key is read from
  stdin's first non-empty line, with UTF-8 BOM stripped (PowerShell
  pipe-safe). Tokens never appear in shell history.

### Storage (single SQLite file, no on-disk config)

- `~/.a2a/credentials.db` (Unix 0600 / Windows user-private ACL):
  - `profiles` table — plaintext API keys (per the trust-the-local-
    user threat model; no encryption / KDF / master password).
  - `meta` table — currently only `default_profile`.
  - `model_aliases` table — user-global model alias registry,
    shared across all a2a projects on this machine.
- No project-level `.a2a/config.toml`, no bundled
  `default-config.toml`, no `--config` / `A2A_CONFIG` override.
  Every value that pre-r20 lived in `[defaults]` is now a
  build-time constant in `crate::defaults`.

### Consultation (`ask`)

- Parallel `cursor-agent` subprocesses (one per model alias), each
  launched with the resolved profile's API key via
  `CURSOR_API_KEY`.
- Workspace isolation via `readonly_mirror` only: a temp dir
  containing only the prompt frontmatter's `context_files`. There
  is no `scratch` mode and no `--scratch` flag.
- Per-call profile chain via `--profiles a,b,c` (comma-separated,
  ordered, deduped). When omitted, a single-element chain is used
  (resolved default profile per SPEC §5.3). There is no
  `--profile` (singular) and no `--no-fallback` flag — pass
  `--profiles <single>` for the equivalent.
- Three-class error routing (hard-coded keyword match against
  `cursor-agent` stderr; no regex config file):
  - **KeyDead** (401 / billing / quota / etc.): delete the profile,
    advance the chain.
  - **ModelUnavailable** / **Unknown**: skip the alias entirely.
  - **Transient** (rate-limit / TLS / DNS / timeout): retry the
    same profile up to 3× with 1 s / 3 s / 10 s back-off, using
    `--resume <session_id>` to keep the same Cursor backend chat.
- Cross-account `--resume` is **never** issued — Cursor backend
  silently drops history on cross-account resume but reports
  success, so the KeyDead profile-switch path always sends a fresh
  prompt with a continuation prefix (SPEC §8.5 / §8.5.1).
- Hard 15-minute timeout per `cursor-agent` invocation (covers
  wedged HTTP/2 streams / TLS half-open). Timeout preserves any
  `session_id` and stderr captured so far via shared `Arc<Mutex>`
  slots, so transient-retry routing still has the data it needs.
- "Store drained" guard: when KeyDead deletes the last profile,
  every in-flight alias bails immediately and the orchestrator
  prints a recovery banner instead of letting peer aliases waste
  more cursor-agent calls.
- Raw-answer audit: every consultation persists each model's
  markdown answer plus a `meta.toml` (alias, cursor_model, mode,
  profile_used, fallback_chain, success, elapsed_ms,
  session_ids[], last_session_id, fallback_attempts[], optional
  `[models.budget]` from `--log-budget`).

### Templates / Cursor IDE integration

- `a2a init [--path <project>] [--force]` writes five templates
  to **two** locations each:
  - `<project>/.a2a/template/<stage_rel>` — staged audit copy.
  - `<project>/<dst_rel>` — live copy under
    `.cursor/skills/{a2a,a2a-operator,a2a-setup-guide}/SKILL.md`,
    `.cursor/rules/40-a2a-protocol.mdc`, and
    `.cursor/templates/a2a-prompt-template.md`.
- All templates are baked into the binary via `include_str!`
  (no on-disk install dir, no `tool_install_dir()` walk). A
  `{{A2A_VERSION}}` placeholder is substituted at install time.
- The three skills together cover the agent flow:
  - `a2a-multi-ai-consult` — *when* to consult multiple models,
    how to synthesize, the AskQuestion-or-bust rule.
  - `a2a-operator` — natural-language → CLI translation for
    everyday operations, with confidentiality rules around
    tokens.
  - `a2a-setup-guide` — first-time-setup wizard triggered by
    the literal user message `a2a_guide`.
- `a2a init --force` is the single-shot way to refresh installed
  templates after upgrading the binary. There is no `a2a sync`
  subcommand.

### Per-command stderr nudge

- When `cursor-agent` is missing from PATH, every subcommand
  except `doctor` / `status` / no-subcommand emits a one-line
  stderr warning pointing at <https://cursor.com/cli>. `doctor`
  and the welcome wizard print their own dedicated reachability
  block, so the warning is suppressed there.

### Housekeeping

- `a2a list` / `a2a clean [--older-than <duration>] [--yes]` for
  consultation-history hygiene. Each `ask` also kicks off a
  detached, best-effort 7-day prune of stale consultation dirs.
- `a2a reset models [--yes]` wipes the SQLite `model_aliases`
  table; `a2a reset credentials [--yes]` deletes
  `~/.a2a/credentials.db` entirely.

### Dependencies

`anyhow`, `chrono`, `clap`, `dialoguer`, `directories`,
`rusqlite` (bundled SQLite — no system dep), `serde`, `serde_json`,
`tempfile`, `thiserror`, `tokio`, `toml`, `tracing` +
`tracing-subscriber`, `uuid`, `walkdir`, `which`.

### Known limitations

- No streaming output forwarded to the user — raw answer files are
  written after each `cursor-agent` subprocess exits. Per-alias
  progress lines (`received first response (streaming...)`,
  `still receiving... +N chars`, `still alive (...)`) are printed
  during the run.
- Budget estimation is character-count-based (proxy for tokens);
  per-model token-aware estimation is not implemented.
- Tested primarily on Windows. macOS / Linux paths are written
  cross-platform but have had less wall-clock testing.
