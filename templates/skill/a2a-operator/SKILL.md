---
name: a2a-operator
description: Translate user natural-language requests into a2a CLI invocations. Use when the user says things like "add an a2a token", "set my Cursor API key", "register a new model", "switch default model", "list configured models", "use account X for this consultation", "reset model aliases", "diagnose a2a" — i.e. when the user is asking the agent to *configure* or *operate* a2a, NOT when the user is asking a hard design question that should trigger the consultation flow itself (that flow is in the sibling `a2a` skill).
---

<!-- a2a template version: {{A2A_VERSION}} (bundled with the a2a binary; do not hand-edit. Run `a2a init --force` after upgrading a2a to refresh this file.) -->

# Skill: a2a operator (natural-language → CLI bridge)

This skill is auto-installed by `a2a init`. It teaches the agent how to map
common conversational requests to `a2a` CLI commands, with the right
non-interactive flags and the right confidentiality precautions.

The companion skill at [`.cursor/skills/a2a/SKILL.md`](../a2a/SKILL.md)
covers a different question — **when** to consult multiple models and how
to synthesize their answers. The third sibling at
[`.cursor/skills/a2a-setup-guide/SKILL.md`](../a2a-setup-guide/SKILL.md)
handles **first-time setup**, triggered when the user types the
literal word `a2a_guide`. This one — `a2a-operator` — covers the
**ongoing** natural-language → CLI translation for everyday operations.

## What r20 changed (read first if you're a returning agent)

- **No more `.a2a/config.toml`**. All persistent state — profiles + the
  meta `default_profile` + model aliases — lives in
  `~/.a2a/credentials.db` (SQLite, single file, user-global).
- **Model aliases are user-global**, not project-scoped. `a2a models
  add opus ...` registers `opus` for every a2a project on this machine.
- **Fallback chains are CLI-time-explicit**. There is no per-alias
  profile binding (`[models.<alias>] profile = ...`), no per-alias
  fallback chain, no `[fallback] default_chain`. To use multiple
  profiles for one consultation, pass `a2a ask --profiles a,b,c` — the
  list is the chain (head fails → try next).
- **`a2a ask` without `--models` runs the FIRST-ADDED alias** (lowest
  `created_at` in `model_aliases`). There is no `set-defaults`
  subcommand and no "default_models is a list" concept.
- **No encryption / disable / hash-dedup features**. Don't propose
  `auth disable|enable|health|init --encrypted|export`; those are
  permanently excluded by SPEC §2 / §17.1.

## Confidentiality rules (always apply)

- **Never echo a token / API key back into chat.** When the user pastes
  one, you have already received it; the user has it in their clipboard
  or scrollback. Do not include it in your reply, in comments, in
  `console.log`, or anywhere else.
- **Never write tokens to disk in plaintext yourself.** Pipe them
  through `a2a auth add --from-stdin` so a2a's SQLite layer is the
  single sink. SPEC §4.1: keys live as plaintext in `~/.a2a/
  credentials.db` (file is 0600 on Unix; user-private ACL on Windows).
  This is by design — a2a's threat model trusts the local user.
- **Never put a token in a command-line argument.** Shell history /
  ps-style logs would leak it. Always pipe via stdin (`echo` /
  `Set-Content` / heredoc / `$'\n'` etc.).
- **Mask in confirmations.** When confirming "stored profile X", show
  only the masked form (`a2a auth show <name>` does this for you —
  first 4 + last 4 characters with `****` in between).

## Common intent → command mapping

### "Add / register / set my Cursor API key" / "添加 a2a 的 token"

User example: `添加 a2a 的 token：crsr_e9122310e7808d7bd38187648427a2316d172e1b58ce584f90b9c6fea13aedd8`

What to do:

1. Extract the token. Do NOT echo it back.
2. Pick a profile name. If the user did not specify, ask via
   `AskQuestion` with reasonable defaults (`default`, `personal`,
   `team`, or extract from context: "my team account" → `team`). If
   the user said something that suggests a name ("my personal one",
   "team Pro+"), use that.
3. Pipe the token via stdin:

   PowerShell:
   ```powershell
   "<TOKEN>" | a2a auth add <profile-name> --from-stdin --note "<short note>"
   ```

   bash / zsh:
   ```bash
   printf '%s\n' '<TOKEN>' | a2a auth add <profile-name> --from-stdin --note "<short note>"
   ```

4. Reply with confirmation only — show the masked form by quoting
   `a2a auth show <profile-name>` (its output is already masked).

### "Update / replace key for profile X" / "更新 X 的 token"

```
"<NEW-TOKEN>" | a2a auth update <profile> --from-stdin
```

The store has no encryption / disable state to clear — it's a plain
column update.

### Handling auth-related errors

`a2a auth add` only enforces one constraint that the agent can hit:
**profile name uniqueness**.

> "profile name '<name>' is already in use"

This says the **profile name** is taken. It does NOT say the stored
key matches the one the user just gave you — a2a never exposes
plaintext keys, so neither agent nor user can compare them.

Surface neutrally via `AskQuestion`:

- **Overwrite** the existing profile's key (`a2a auth update <name>
  --from-stdin`). Useful for token rotation or fixing a broken key.
- **Keep** the existing profile and store the new key under a different
  name (`a2a auth add <other-name> --from-stdin`).
- **Cancel** and leave everything as-is.

There is no "API key already stored under another profile" error —
SPEC §2.3 explicitly drops the API-key hash dedup, so the same token
can live in multiple profiles. (This is intentional: the user might
want `default` + `personal` to point at the same Pro+ account.)

There is no `auth rename` subcommand; if the user asks to rename, do
`auth remove <old> --yes` then `auth add <new> --from-stdin` with the
same token.

### "Set default profile to X" / "默认用 X 账号"

```
a2a auth use <profile>
```

Stores `<profile>` as `meta.default_profile` in `~/.a2a/credentials.db`.
Resolution order at `a2a ask` time: `meta.default_profile` → the
literal profile named `"default"` → the first profile by `created_at`.
SPEC §5.3.

### "Show me what accounts I have" / "列一下账号"

```
a2a auth list
```

Display the output verbatim. Do not paraphrase masked keys.

### "Remove a profile" / "删掉 X"

```
a2a auth remove <name>           # interactive confirm
a2a auth remove <name> --yes     # skip confirm
```

If `<name>` is the current default profile, a2a clears
`meta.default_profile` automatically.

### Listing / inspecting models

Two distinct commands serve two different questions:

- **"What model aliases are registered on this machine?"** →
  `a2a models list` (or `a2a models list --verbose`). Reads the
  user-global `model_aliases` SQLite table. The `*` mark indicates
  the **first-added** alias — that's what `a2a ask` runs when the
  user omits `--models`.
- **"What models is my Cursor account entitled to?"** →
  `a2a models available [--profile <name>]`. Calls `cursor-agent
  --list-models` under the resolved profile. The `@` mark indicates
  an upstream model id is already referenced by some local alias.

Use `available` to discover model ids before adding aliases. The agent
should treat `available` as the source of truth for model id strings;
do not fabricate model ids from memory — Cursor's catalog evolves.

#### Cursor model id naming convention

Cursor encodes capability tags directly into the model id, so a2a does
not need separate "thinking strength" / "context size mode" flags.
Read the segments off the id:

| segment | meaning |
| --- | --- |
| `low` / `medium` / `high` / `xhigh` (or `extra-high`) / `max` | size / capacity / reasoning tier |
| `thinking` | extended reasoning variant (vs the non-thinking baseline) |
| `fast` | reduced-latency variant of the same model |

When the user asks for "the smartest X" / "the fast variant" /
"thinking mode", run `a2a models available`, filter by family, and
use the segment table to map intent to a concrete id.

### "Add a new model alias" / "let me consult model X under name Y"

```
a2a models add <alias> --model <cursor-id> [--mode plan|agent] \
    [--thinking-hint "..."] [--description "..."] [--force]
```

There is **no** `--profile` flag (per-alias profile binding was
dropped in r20; pass `--profiles` to `a2a ask` at call time instead).
There is **no** `--as-default` flag (the first-added alias is the
default automatically; rotate aliases via `--force` to preserve the
original `created_at`, or by removing + re-adding to give a new
alias the "first-added" position).

Workflow when the user names something:

1. **User names a specific cursor model id** (verify it appears in
   `a2a models available`): pick a short alias (e.g. derive from the
   id tail) and confirm via `AskQuestion` if ambiguous; then run
   `a2a models add`.
2. **User names a capability, not an id** ("the smartest model in
   family X", "the fast variant of Y"): run `a2a models available`
   first, filter by family or capability tag, present candidates via
   `AskQuestion`, then add the picked one.
3. **User names a model id that does NOT appear in `available`**:
   tell them so. Don't add an alias to a model their account cannot
   use — `a2a ask` will fail later with `model_unavailable`
   classification.
4. **User asks for a list/catalog of "all models"**: do NOT enumerate
   from memory. Run `a2a models available` and quote (or summarize)
   its output. The catalog is account- and date-specific.

If `--description` is supplied, it shows in `a2a models list`. Compose
one from the user's intent ("Opus thinking max") to keep `list`
readable.

### "Change a model alias" / "edit alias X" / "rename"

```
a2a models set <alias> [--model X] [--mode X] \
    [--thinking-hint X] [--description X]
```

The `set` command updates only the columns it's given; absent flags
keep the existing values. There is **no** `--profile` /
`--fallback-profiles` (those concepts were dropped — fallback is
CLI-time via `a2a ask --profiles`).

a2a has no `rename` subcommand; if the user asks to rename, do `a2a
models add <new> --model <old's cursor-id> --description "..."` then
`a2a models remove <old> --yes`.

### "Remove a model alias"

```
a2a models remove <alias>           # interactive confirm
a2a models remove <alias> --yes     # skip confirm
```

Removes the row from the user-global `model_aliases` SQLite table. If
the user asks to remove an alias that doesn't exist, the CLI tells
them. a2a never touches Cursor's upstream catalog.

### "Use models X and Y for this consultation" / "本次只问 X + Y"

This is a per-call override:

```
a2a ask <topic> --prompt-file <path> --models <a>,<b>
```

There is no project-default-models concept anymore; if the user wants
a fixed default set, they have two options:

- Add the desired alias FIRST (before others) so it's the
  first-added → automatic default.
- Always pass `--models` explicitly at call time.

### "Use my personal account this once" / "force this run on team only"

```
a2a ask <topic> --prompt-file <path> --profiles personal
```

`--profiles` is the **chain** (comma-separated). For a single
profile, just pass one name (`--profiles personal`) — no fallback.
For a chain (try `personal` first, fall back to `team` on KeyDead):
`--profiles personal,team`.

### "Try a chain — fall back if my main account fails"

```
a2a ask <topic> --prompt-file <path> --profiles default,personal,team
```

SPEC §6.3: the chain head goes first; KeyDead (401 / billing / quota)
deletes that profile and advances to the next; ModelUnavailable /
Unknown skip the whole alias; Transient retries up to 3× on the
same profile. Without `--profiles`, the chain has one element
(the resolved default profile per §5.3).

### Default behaviour: agent mode + read-only directive

By default `a2a ask`:

- Spawns cursor-agent in **`agent` mode** (allows file writes inside
  the isolated readonly_mirror tempdir, NOT the user's project).
- **Prepends a read-only directive** to every prompt telling the
  consulted model: don't modify project files; print the answer to
  stdout; supporting notes can go inside `.a2a/` of the mirror.

Workspace isolation (`readonly_mirror`) confines side effects to a
tempdir even if the model ignores the directive. There is no
`scratch` mode (SPEC §2.2 — git worktree isolation is permanently
excluded).

To suppress the directive (rare; e.g. when the model is asked to
generate raw output that would conflict with the directive's
formatting):

```
a2a ask <topic> --prompt-file <path> --no-readonly-prefix
```

To force plan mode for a specific alias (cursor-agent then refuses
all writes at the FS level):

```
a2a models set <alias> --mode plan
```

Or override per-call:

```
a2a ask <topic> --prompt-file <path> --mode plan
```

### Cursor process-level sandbox (`--sandbox`)

Separate from a2a's own workspace isolation, Cursor itself implements
a **process / kernel-level sandbox** — Seatbelt on macOS,
Landlock+seccomp on Linux, WSL2 on Windows — controlled by
`~/.cursor/sandbox.json` / `<workspace>/.cursor/sandbox.json` and the
cursor-agent `--sandbox` flag.

a2a does **not** pass `--sandbox` by default; cursor-agent then uses
the user's `sandbox.json` / IDE setting. If the user explicitly
requests a hardened run, pass through:

```
a2a ask <topic> --prompt-file <path> --sandbox enabled
```

This is appropriate when:

- The user says "run with sandbox" / "sandboxed" / "extra isolation".
- The consulted model is expected to run untrusted shell commands and
  the user wants kernel-level containment instead of relying solely
  on a2a's workspace tempdir.

`--sandbox disabled` is the inverse and rarely needed; only reach for
it if a sandboxed run is failing because the model legitimately needs
broad network / system access.

When in doubt, do NOT pass `--sandbox`; let the user's per-machine
`sandbox.json` decide.

### Streaming progress

`a2a ask` runs cursor-agent in stream-json mode and prints two kinds
of progress lines to stdout:

1. `[<alias>] received first response (streaming...)` — first
   assistant token arrived. Useful so the user knows the request is
   not stuck when it might otherwise sit silent for ~10 s.
2. `[<alias>] still receiving... +N chars (total M chars in last
   10s)` — periodic char-count tick every 10 seconds, **only** when
   the count has changed since the previous tick (silent during stalls
   or end-of-response).

Plus an "alive" tick when the model has been silent for ~30 s
(usually means it's reasoning / tool-calling).

### "Skip a2a, just give me your candidates"

The user is opting out of consultation for this turn. Do NOT consult.
Drop straight to `AskQuestion` with the candidates the agent has
already considered (per the rules in
`.cursor/rules/40-a2a-protocol.mdc`).

### "Check that a2a is set up correctly" / "Diagnose a2a"

```
a2a doctor
a2a status
a2a auth list
a2a models list
```

Run all four; quote outputs verbatim; if `cursor-agent NOT in PATH`
is reported, point the user at https://cursor.com/cli for
installation.

For agent-mode consumption (e.g. you've been told to run a2a
non-interactively to discover state):

```
a2a --agent
```

That prints a structured `[health]` block + an `[next-step for you,
the agent]` block worded as imperatives. The agent flag never
prompts for Y/n and never pauses for Enter — safe to capture stdout
in a single read.

### "Clean up old consultations"

```
a2a list                          # show what's there
a2a clean --older-than 30d        # interactive confirm; --yes to skip
```

### "Refresh / update the bundled Cursor templates" / "skill 升级"

After upgrading the `a2a` binary itself, refresh the project's
installed templates with:

```
a2a init --force
```

This re-writes `.cursor/skills/{a2a,a2a-operator,a2a-setup-guide}/
SKILL.md` + `.cursor/rules/40-a2a-protocol.mdc` +
`.cursor/templates/a2a-prompt-template.md` from the freshly-built
binary. The `.a2a/template/...` staged copies are always overwritten;
the live `.cursor/...` files are overwritten only with `--force`.
There is no separate `a2a sync` command (SPEC §3 — that surface was
removed in r20; `init --force` is the single-shot replacement).

### "Reset" operations

Two reset targets — always confirm what scope is intended before
running:

| User says | Subcommand | What it does |
| --- | --- | --- |
| "remove all model aliases" / "start fresh with models" | `a2a reset models [--yes]` | Wipes the user-global `model_aliases` SQLite table. Profiles + `meta.default_profile` are NOT touched. |
| "wipe all API keys" / "remove all accounts" | `a2a reset credentials [--yes]` | Deletes `~/.a2a/credentials.db` entirely (profiles + meta + aliases). Irreversible. |

There is no `a2a reset config` and no `a2a reset defaults` (project
toml and the `default_models` list both no longer exist; SPEC §17.1
#17). Use `AskQuestion` to confirm scope before any reset that
cannot be easily undone (especially `credentials`).

## When NOT to use this skill

This skill is for **operating the a2a tool**. It is not a hook for
hard design decisions. If the user asks "should we use Map or Object
for this?" / "design X versus design Y?", read the sibling skill
[`a2a/SKILL.md`](../a2a/SKILL.md) — that's the consultation flow,
with its own rules in `.cursor/rules/40-a2a-protocol.mdc`.

If the user types the literal word `a2a_guide` (with underscore) as
their entire message, that's the first-time-setup wizard's trigger;
defer to [`a2a-setup-guide/SKILL.md`](../a2a-setup-guide/SKILL.md).

If the project also installs a project-specific overlay skill
(typically `<projectname>-a2a-overlay/SKILL.md`), follow that
overlay's additional requirements when running the consultation flow.

## Disambiguation: ambiguous user requests

If you're not sure whether a request is "operate a2a" or "consult
about a hard question", lean toward asking the user a single short
`AskQuestion` to clarify. Examples:

- "switch a2a to <alias>" → ambiguous (which alias to make
  first-added? this run only via `--models`?). Ask.
- "add my token" → ambiguous if multiple profiles exist (which one
  is this for?). Ask, unless the immediately preceding turn already
  named one.
- "ask <alias>" → ambiguous (consult <alias> on a question vs.
  re-add <alias> to be the first-added default). Ask.
- User names a model id that doesn't appear in `a2a models
  available` → tell the user the catalog doesn't show it; suggest
  running `a2a models available` so they pick a real id.

## Checklist before invoking any a2a auth command from a chat context

- [ ] Token received from the user is NOT echoed in your reply
- [ ] Token is fed via `--from-stdin` pipe, not as a positional /
      `--key=...` flag or shell argument
- [ ] Profile name disambiguated (asked the user or extracted from
      context)
- [ ] After the command, run `a2a auth list` so the user can see the
      result (the output already masks)
- [ ] Confirmation message uses the masked form, never the raw token
