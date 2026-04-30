---
name: a2a-setup-guide
description: First-time setup wizard for a2a, triggered exclusively when the user sends the single word "a2a_guide" (case-insensitive, with the underscore) in a Cursor chat. Walks the user through registering a Cursor API key (`a2a auth add`) and registering at least one model alias (`a2a models add`) so this project is ready for `a2a ask`. Do NOT activate on free-form questions about a2a — those belong to the sibling `a2a-operator` skill, or should be answered directly.
---

<!-- a2a template version: {{A2A_VERSION}} (bundled with the a2a binary; do not hand-edit. Run `a2a init --force` after upgrading a2a to refresh this file.) -->

# Skill: a2a setup guide

This skill is auto-installed by `a2a init`. It activates **only** when
the user's message is the single literal trigger keyword
`a2a_guide` (case-insensitive; possibly surrounded by whitespace).
Anything else — including ordinary questions about a2a, "how do I
add a token?", "what does a2a do?", or even messages that contain
`a2a_guide` as part of a larger sentence — should NOT activate this
skill. Use the sibling `a2a-operator` skill for natural-language
operations, or answer directly.

The keyword was deliberately chosen with an underscore so the trigger
is unambiguous: a user typing `a2a_guide` is signalling "start the
setup wizard right now"; a user mentioning `a2a` alone is not.

## Wizard flow

When triggered, run these steps **in order**, asking the user via
`AskQuestion` between each. Keep responses concise and never echo
API keys back into chat.

### Step 1 — verify environment

Run these three CLI commands, capture stdout, and report the
relevant status to the user:

  - `a2a doctor`     — checks cursor-agent reachability + version
  - `a2a auth list`  — current registered profiles
  - `a2a models list` — current registered model aliases

If `a2a doctor` reports `cursor-agent NOT in PATH`, instruct the
user to install Cursor CLI from https://cursor.com/cli before
continuing, then stop the wizard. Do not attempt to install it
on their behalf.

### Step 2 — register an API key (skip if a profile already exists)

If `a2a auth list` shows zero profiles:

  1. Tell the user (verbatim or paraphrased): "Please paste your
     Cursor API key (starts with `key_` or `crsr_`). I will pipe
     it into a2a so it never appears in shell history or chat
     logs."
  2. Wait for the user's next message. **CRITICAL: do NOT echo the
     key back into chat anywhere.**
  3. Pipe the key via stdin to `a2a auth add`. PowerShell:

     ```powershell
     "<KEY>" | a2a auth add default --from-stdin --note "set up via a2a_guide"
     ```

     bash / zsh:

     ```bash
     printf '%s\n' '<KEY>' | a2a auth add default --from-stdin --note "set up via a2a_guide"
     ```

  4. Run `a2a auth list` and show the masked output to confirm.
  5. Run `a2a auth use default` to set it as the default profile.

If profiles already exist, skip this step. Tell the user:
"Found N profile(s) already; using `<default>` as the default."

### Step 3 — register a model alias (skip if any alias exists)

If `a2a models list` shows zero aliases:

  1. Run `a2a models available` to see what the user's account can
     use. The output is a list of `<id> - <description>` lines.
  2. From that list, propose 1–3 sensible default aliases. A
     common starting set, when available on the account:
       - `opus`   → strongest Claude variant in the catalog
       - `gpt5`   → strongest GPT-5 variant
       - `gemini` → strongest Gemini variant
     Pick model ids that actually appear in `a2a models available`;
     do not invent ids from memory.
  3. Use `AskQuestion` to let the user choose 1+ aliases to
     register, with the proposed defaults pre-checked.
  4. For each picked id, run:

     ```
     a2a models add <alias> --model <cursor-id> --description "<short human-friendly label>"
     ```

  5. Run `a2a models list` to confirm. The first-added alias
     becomes the default for `a2a ask` (no `--models` argument).

If aliases already exist, skip this step.

### Step 4 — finish

Tell the user, briefly:

  - "Setup complete. To run a consultation:

         a2a ask <topic-slug> --prompt-file <path-to-prompt.md>"

  - Point them at `.cursor/templates/a2a-prompt-template.md` as a
    starting prompt template (frontmatter explains the required
    fields).
  - Mention the sibling skills:
      - `a2a-multi-ai-consult` — when the agent should consult
        multiple models on hard design decisions.
      - `a2a-operator` — natural-language → CLI translation for
        ongoing operations (add/remove tokens, change defaults,
        list models, etc.).

Do NOT proactively run an `a2a ask` "demo" unless the user asks for
one — first-time setup is about getting credentials and aliases in
place, not about burning quota on an example.

## Confidentiality (re-stated; this is the most important rule)

- Never echo an API key back into chat. The user has it in their
  clipboard / scrollback already; repeating it leaks it into chat
  logs that may be persisted by Cursor or shared with others.
- Always pipe keys via `--from-stdin`, never via a positional
  argument or `--key=...` (those land in PowerShell `Get-History`
  and similar shell history sinks).
- When confirming, show only the masked form. `a2a auth show
  <name>` masks for you (first 4 + last 4 chars).

## Anti-patterns (forbidden)

- Activating on messages that merely contain "a2a_guide" as part
  of a larger sentence (e.g. "what is a2a_guide?") — answer those
  with a one-line description, do not run the wizard.
- Activating on plain "a2a" — that's `a2a-operator` territory, or
  the user double-clicked a2a.exe and got the welcome wizard.
- Echoing API keys, even partially, even masked-by-you — let
  `a2a auth show` do any masking.
- Running `a2a ask` proactively to "test" the setup — quota burn.
  Setup ends with "ready"; the user runs the first ask themselves.
- Skipping `AskQuestion` and silently registering aliases the user
  hasn't approved.
- Re-running steps that don't apply — Step 2 / Step 3 are skip-if-
  already-configured; do not delete-and-recreate working state.
