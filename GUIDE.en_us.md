# a2a — User Guide (English)

> 🌐 Other languages: [中文](GUIDE.zh_cn.md) · [Back to README](README.md)

This is the detailed usage guide for `a2a`. For a quick overview of what a2a is, see [README.md](README.md).

## Table of contents

1. [Installation](#1-installation)
2. [First-time setup (recommended: agent-driven)](#2-first-time-setup-recommended-agent-driven)
3. [Manual CLI setup (alternative)](#3-manual-cli-setup-alternative)
4. [Daily usage](#4-daily-usage)
5. [CLI reference](#5-cli-reference)
6. [Multi-account fallback](#6-multi-account-fallback)
7. [Welcome / agent-mode entry points](#7-welcome--agent-mode-entry-points)
8. [Troubleshooting](#8-troubleshooting)
9. [How a2a stores state](#9-how-a2a-stores-state)

## 1. Installation

### Prerequisites

- **Cursor CLI** (`cursor-agent`). a2a wraps this; you must install it separately. Download from [https://cursor.com/cli](https://cursor.com/cli); verify in a terminal with `cursor-agent --version`.
- **(For building from source only)** Rust 1.85+ via [https://rustup.rs/](https://rustup.rs/).

### Option A — pre-built binary (recommended)

1. Download / build `a2a.exe` (Windows) or `a2a` (macOS/Linux) to a stable location. Suggested: `D:\tools\a2a\a2a.exe` on Windows or `~/.local/bin/a2a` on macOS/Linux.
2. **Double-click** `a2a.exe` (Windows) — or run `./a2a` from a terminal (Unix). The first invocation triggers the welcome wizard: it detects whether the binary's directory is already on your **user-level PATH**, offers `[Y/n]` to add it if not (Windows: writes `HKCU\Environment\Path` via PowerShell; Unix: prints the `export PATH=...` line for you to add to your shell rc), and probes for `cursor-agent` (or points you at the Cursor CLI installer if missing).
3. **Close the current terminal and open a new one.** PATH changes only take effect in newly-launched processes.
4. From any directory: `a2a --version` should now work.

### Option B — build from source

```bash
git clone <wherever the repo lives>
cd a2a
cargo build --release
```

Output: `target/release/a2a.exe` (Windows) or `target/release/a2a` (Unix). On Windows the bundled `build.ps1` additionally copies the binary to a stable `D:\tools\a2a\a2a.exe` after each build, so you can keep `D:\tools\a2a` on PATH instead of the cargo `target/` directory.

## 2. First-time setup (recommended: agent-driven)

Once a2a is on PATH and Cursor CLI is installed, the rest of setup is driven by Cursor's **main agent**. You don't have to know any CLI flags.

1. Open Cursor on your project. Start a fresh chat.
2. Paste one of these prompts and send:
  English:
  > Run `a2a --agent` in the terminal and follow its output.
   中文:
  > 在终端运行 `a2a --agent`，根据它的输出执行下一步。
3. The agent will:
  1. Run `a2a --agent` and read the structured `[health]` block to discover the current state (whether the project is initialised, whether `cursor-agent` is reachable, how many profiles + aliases are registered).
  2. Run `a2a init --path <workspace>` to install three Cursor skills + one rule + one prompt template into the project.
  3. Read the imperative `[next-step]` block from `a2a init`'s output and tell **you** to do the next thing.
4. **Restart Cursor.** (Close every window, then reopen on the project.) Cursor only loads new skills under `.cursor/skills/` on startup, so this restart is mandatory.
5. Open a **new chat**. As your first message, type the literal word — by itself, no extra context:
  ```
   a2a_guide
  ```
   The freshly-installed `a2a-setup-guide` skill activates and walks you through:
  1. Pasting your Cursor API key. The agent pipes it via stdin (`a2a auth add ... --from-stdin`) so it never appears in chat logs or shell history.
  2. Choosing 1–3 model aliases (the agent runs `a2a models available` to see what your account can use, then proposes sensible defaults like `opus` / `gpt5` / `gemini`).
6. Done. From now on, the main agent uses the `a2a-multi-ai-consult` skill (when to consult) and `a2a-operator` skill (how to translate natural-language requests into a2a commands) automatically. You can keep working normally; whenever the agent hits a hard architectural decision it'll trigger `a2a ask` on its own and synthesize the answers for you.

## 3. Manual CLI setup (alternative)

If you'd rather not involve Cursor's agent for setup, do it by hand:

```bash
# 1. Get an API key from https://cursor.com/dashboard → Integrations.
#    a2a will prompt for it (input is hidden):
a2a auth add default

# 2. (If you have other accounts) register them too:
a2a auth add personal
a2a auth add team
a2a auth use default          # set the active default profile

# 3. List what models your Cursor account can access:
a2a models available

# 4. Register one or more aliases. The first one you register is
#    the default for `a2a ask` (when --models is omitted):
a2a models add opus --model claude-opus-4-7-thinking-xhigh \
    --description "Opus 4.7 1M Thinking Extra High"
a2a models add gpt5 --model gpt-5.5-extra-high \
    --description "GPT-5.5 1M Extra High"
a2a models add gemini --model gemini-3.1-pro \
    --description "Gemini 3.1 Pro"

# 5. (Optional) Install the Cursor skills + prompt template into a
#    project. Skip this step if you only want to use the CLI:
cd /path/to/your/project
a2a init
```

To verify everything: `a2a doctor` (or `a2a --agent` for an agent-friendly machine-readable version).

## 4. Daily usage

### 4.1 Writing a prompt file

Prompt files are markdown with a YAML frontmatter. Minimum fields:

```markdown
---
topic: cache-design
context_files:
  - SPEC.md
  - .cursor/rules/
  - src/cache/lru.rs
  - src/cache/eviction.rs
---

# Question

Should our LRU cache use a doubly-linked list with a HashMap, or
move to a slot-based approach with `slotmap`?

## Constraints

- Hot path is read-heavy (~20:1 read:write ratio).
- Memory ceiling: 256 MB cache, ~10 KB per entry, ~25k entries max.

## Candidates already considered

### (a) Current — DLL + HashMap
...

### (b) `slotmap`-based
...
```

The frontmatter `context_files` lists the project files the consulted models will see. **Always include the project's governance documents** (e.g. `SPEC.md`, `AGENTS.md`, `.cursor/rules/`) plus the files that are directly relevant to the question. The bundled `a2a-multi-ai-consult` skill (installed by `a2a init`) has the full mandate including a pre-flight checklist.

`.cursor/templates/a2a-prompt-template.md` (installed by `a2a init`) is a template you can copy.

### 4.2 Running a consultation

```bash
a2a ask <topic-slug> --prompt-file <path-to-prompt.md>
```

When `--models` is omitted, a2a runs only the **first-added** alias. To consult several models at once:

```bash
a2a ask cache-design \
    --prompt-file consultations/2026-04-30-cache.prompt.md \
    --models opus,gpt5,gemini
```

The CLI prints per-alias progress lines as the models stream output:

```
[opus]   profile=default → calling cursor-agent (phase=fresh)
[gpt5]   received first response (streaming...)
[gpt5]   still receiving... +210 chars (total 210 chars in last 10s)
[opus]   still alive (no new streamed text in 30s; cursor-agent likely thinking / tool-calling)
[gpt5]   profile=default → OK (148.0s)
[opus]   profile=default → OK (704.7s)
...
Done. 3 succeeded / 0 failed.
```

### 4.3 Reading the output

Each consultation creates a directory under `<project>/consultations/`:

```
consultations/20260430-225014-371-cache-design-d0a8b9/
├── prompt.md             # copy of the prompt that was sent
├── opus.answer.md        # Opus's raw markdown answer
├── gpt5.answer.md
├── gemini.answer.md
└── meta.toml             # which profile, session_id, fallback chain, timing
```

If you ran `a2a ask` from inside Cursor's chat, the main agent will read each `<alias>.answer.md`, build a synthesis (agreement / disagreement / new candidates), and ask you to pick via `AskQuestion`. You can always open the raw `*.answer.md` files yourself to verify.

`meta.toml` has structured fields:

```toml
topic = "cache-design"
created_at = "2026-04-30T22:50:14Z"
a2a_version = "0.1.0"
command_line = "a2a ask cache-design --prompt-file ..."

[[models]]
alias = "opus"
cursor_model = "claude-opus-4-7-thinking-xhigh"
mode = "agent"
profile_used = "default"
fallback_chain = ["default"]
success = true
elapsed_ms = 704735
answer_path = "..."
session_ids = ["abc-..."]
last_session_id = "abc-..."

[[models.fallback_attempts]]
profile = "default"
success = true
elapsed_ms = 704735
session_id = "abc-..."
```

You can resume the same Cursor backend chat manually with `cursor-agent --resume <session_id>` if you want to continue a specific model's thread.

## 5. CLI reference

### `a2a` (no subcommand)

Welcome wizard for human use. PATH check + cursor-agent check + quick-start hints. Pauses for Enter at the end if stdin is a TTY (so the console doesn't snap shut on double-click).

### `a2a --agent`

Same situational checks, but for AI-agent consumption: never pauses, never prompts, emits a structured `[health]` block + an imperative `[next-step]` block. Cursor's main agent invokes this to discover state and decide what to tell the user.

### `a2a init [--path <project>] [--force]`

Installs the bundled Cursor templates into a project. Each template is written to two locations: `<project>/.a2a/template/<rel>` (audit copy, always overwritten) and `<project>/<dst_rel>` (live copy under `.cursor/...`, respects `--force`).

### `a2a ask <topic> --prompt-file <path> [flags]`

Run a consultation. Common flags:


| Flag                         | Default                  | Meaning                                                                                                        |
| ---------------------------- | ------------------------ | -------------------------------------------------------------------------------------------------------------- |
| `--models a,b,c`             | first-added alias        | Aliases to consult (comma-separated).                                                                          |
| `--profiles a,b,c`           | resolved default profile | Profile chain for this run. KeyDead deletes the head and advances; transient errors retry on the same profile. |
| `--mode agent|plan`          | per-alias `default_mode` | cursor-agent's `--mode` passthrough.                                                                           |
| `--sandbox enabled|disabled` | (cursor-agent default)   | passthrough.                                                                                                   |
| `--no-readonly-prefix`       | off                      | Skip the read-only directive injection.                                                                        |
| `--dry-run`                  | off                      | Print the cursor-agent commands; don't run.                                                                    |
| `--budget-only`              | off                      | Print a char-count estimate; don't run.                                                                        |
| `--log-budget`               | off                      | Attach a `[models.budget]` table to `meta.toml`.                                                               |


### `a2a auth ...`

```
a2a auth add <name> [--from-stdin] [--note <text>]
a2a auth list
a2a auth use <name>
a2a auth show <name>           # masked: first 4 + last 4 chars
a2a auth remove <name> [--yes]
a2a auth update <name> [--from-stdin]
```

`--from-stdin` reads the API key from stdin's first non-empty line (UTF-8 BOM stripped). Strongly recommended for any scripted / agent-driven add — tokens never appear in shell history.

### `a2a models ...`

```
a2a models                                  # = list (default)
a2a models list [--verbose]
a2a models available [--profile <name>]
a2a models add <alias> --model <cursor-id> \
    [--mode plan|agent] [--thinking-hint X] \
    [--description X] [--force]
a2a models set <alias> [--model X] [--mode X] \
    [--thinking-hint X] [--description X]
a2a models remove <alias> [--yes]
```

`--force` on `add` re-defines an existing alias while preserving its original `created_at`, so the first-added-default doesn't shuffle when you rotate aliases.

### `a2a doctor` / `a2a status`

`doctor` checks: a2a version + OS/arch + cursor-agent reachability + profile / alias counts + project-initialised flag. `status` is a shorter version focused on cursor-agent login state.

### `a2a list` / `a2a clean`

```
a2a list                                # past consultations in this project
a2a clean [--older-than 30d] [--yes]    # prune (interactive by default)
```

Each `a2a ask` also kicks off a detached, best-effort 7-day prune of stale consultation dirs in the same project.

### `a2a reset ...`

```
a2a reset models [--yes]         # wipe the SQLite model_aliases table
a2a reset credentials [--yes]    # delete ~/.a2a/credentials.db entirely
```

Both are irreversible; both prompt for confirmation unless `--yes`.

## 6. Multi-account fallback

a2a supports running a consultation across a chain of profiles (typically: a primary account + one or two secondary accounts). When the head of the chain hits an account-level failure (`401 Unauthorized`, billing required, quota exceeded, subscription expired, …), a2a:

1. Deletes the head profile from `~/.a2a/credentials.db`.
2. Advances the chain to the next profile.
3. Prints the transition (`[<alias>] profile=X → KeyDead detected; deleting; advancing fallback chain`).
4. Records every attempt in `meta.toml`'s `fallback_attempts`.

Network-level failures (TLS handshake, DNS, rate-limit `429`, `timeout`) are treated as **transient** and retry on the same profile up to 3× with 1 s / 3 s / 10 s back-off. The retry uses `cursor-agent --resume <session_id>` to pick up the same chat on the Cursor backend, so the retried call doesn't re-replay the full prompt.

```bash
# Try `personal` first; fall back to `team`, then `default`:
a2a ask my-topic --prompt-file <path> --profiles personal,team,default
```

When `--profiles` is omitted, a2a uses a single-element chain. The default profile is resolved as:

1. The SQLite `meta.default_profile` (set via `a2a auth use`), if it still exists.
2. Else the literal profile named `"default"`, if it exists.
3. Else the first profile by `created_at` (earliest registered).

If the chain is exhausted for an alias without success, that alias is reported as failed; other aliases continue in parallel.

If the **last** profile in the credentials store gets KeyDead-deleted mid-run, every in-flight alias bails immediately and the orchestrator prints a recovery banner — no point burning further cursor-agent calls against an empty store.

## 7. Welcome / agent-mode entry points

a2a has two zero-subcommand entry points that share most of the logic but differ in interactivity:


| Invocation    | When                                                              | Behaviour                                                                                                                                                                                                 |
| ------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `a2a`         | Human double-click on Windows; or just typing `a2a` in a terminal | PATH check + cursor-agent check + quick-start hints. Asks Y/n to fix PATH issues. **Pauses for Enter** if stdin is a TTY (so the console doesn't snap shut).                                              |
| `a2a --agent` | Cursor's main agent invokes from its terminal tool                | Same situational checks, but never pauses, never prompts. Output is a structured `[health]` block (machine-parseable `key: value` lines) + a `[next-step for you, the agent]` block (imperative English). |


The two modes are idempotent: running them after everything is already configured prints a clean health report and exits without mutating state.

## 8. Troubleshooting

### `cursor-agent NOT in PATH`

a2a depends on Cursor's CLI for the actual model calls. Install from [https://cursor.com/cli](https://cursor.com/cli) and reopen your terminal. `a2a doctor` will confirm.

### `path_installed: no` in `a2a --agent` output

Means the terminal Cursor's agent is using has a **process-level PATH** that doesn't include the directory of the binary it just ran. Most common cause: Cursor was launched **before** your user PATH was modified, so its child terminals still hold the cached env.

**Fix**: close all Cursor windows completely, then reopen on the project. Fresh terminals pick up the latest registry PATH. (`a2a --agent` automatically prints a verbose "tell the user to restart Cursor" block when this state is detected.)

### `stale_a2a_path_entries: N` in `a2a --agent` output

User PATH contains other directories that hold an `a2a.exe` (e.g. `D:\tools\a2a\target\release` from cargo's build dir). Run plain `a2a` (no args) interactively — the wizard offers to remove them in one Y/n.

### `credentials_store: ERROR (...)` in `a2a --agent` output

The SQLite file at `~/.a2a/credentials.db` couldn't be opened (corrupt / locked / permission-denied). `a2a --agent`'s `[next-step]` block routes to a STOP path so Cursor's agent doesn't loop the user through `a2a_guide` (which calls the same SQLite open and would hit the same error).

Inspect the file. If you don't have profiles worth keeping, `Remove-Item ~/.a2a/credentials.db` (Windows) / `rm ~/.a2a/credentials.db` (Unix) and re-run `a2a auth add`.

### `Done. 0 succeeded / N failed.`

Every alias hit a non-recoverable error. Look at the per-alias `[<alias>] profile=...` lines printed during the run; the last 8 lines of `cursor-agent` stderr are inlined for each failed alias. Common causes:

- Account quota exhausted on every profile in the chain.
- All chain profiles have the same broken Cursor session token.
- All requested model aliases are unavailable on the resolved profile (`ModelUnavailable`).

### Re-installing skills after a2a upgrade

```bash
cd /path/to/project
a2a init --force
```

`--force` overwrites the `.cursor/skills/...` files with the embedded versions in the new binary. The audit copies under `<project>/.a2a/template/...` are always refreshed regardless of `--force`. There is no `a2a sync` subcommand — `init --force` is the single-shot replacement.

## 9. How a2a stores state

### The single SQLite file

`~/.a2a/credentials.db` is the **only** persistent state a2a owns. Three tables:


| Table           | Purpose                                                                                                                                                                      |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `profiles`      | Plaintext API keys + a few timestamps. The threat model trusts the local user (the file is 0600 on Unix, user-private ACL on Windows). |
| `meta`          | `default_profile` only (the profile name `a2a auth use` writes).                                                                                                             |
| `model_aliases` | User-global model alias registry, shared across every a2a project on this machine.                                                                                           |


### What `a2a init` creates inside a project

- `<project>/.a2a/` — project marker directory (used by `find_project_root`).
- `<project>/.a2a/template/` — staged audit copies of the bundled templates. Always overwritten by `a2a init`.
- `<project>/consultations/.gitignore` — ignores everything under `consultations/` (per-run dirs are user-private, contain raw answers + meta).
- `<project>/.cursor/skills/{a2a,a2a-operator,a2a-setup-guide}/SKILL.md` — three Cursor skills.
- `<project>/.cursor/rules/40-a2a-protocol.mdc` — protocol rule.
- `<project>/.cursor/templates/a2a-prompt-template.md` — prompt skeleton you copy when writing a new consultation prompt.

### What's hardcoded (no override knob)

The runtime constants below are baked into the binary at build time. Changing them requires rebuilding `a2a` from source:


| Constant                  | Value                                         |
| ------------------------- | --------------------------------------------- |
| `PARALLEL`                | `true` (model aliases run concurrently)       |
| `OUTPUT_ROOT`             | `"consultations"` (project-relative)          |
| `STAGGER_SECS`            | `3` (seconds between successive alias spawns) |
| `INLINE_PROMPT_MAX_BYTES` | `24_000` (above this → indirect prompt)       |

### See also

- [CHANGELOG.md](CHANGELOG.md) — release history.
- [README.md](README.md) — short project overview.
- The three skills installed by `a2a init` (under `<project>/.cursor/skills/`) — agent-side documentation for consultation flow, operations, and first-time setup.

