---
name: a2a-multi-ai-consult
description: Consult multiple AI models in parallel via cursor-agent and synthesize their answers before asking the user to decide. Use when facing a hard design / architecture / spec-lock decision where the agent reasonably believes its own training-time knowledge is insufficient (no clear best answer, multiple candidates with similar trade-offs, or the project's own governance rules say "this kind of decision must be confirmed externally").
---

<!-- a2a template version: {{A2A_VERSION}} (bundled with the a2a binary; do not hand-edit. Run `a2a init --force` after upgrading a2a to refresh this file.) -->

# Skill: a2a multi-AI consultation

This skill is auto-installed by `a2a init` and tells an AI agent how to use
the `a2a` CLI to consult multiple LLMs in parallel before recommending a
solution to the user.

## When to trigger

Trigger automatic consultation **only** when at least one of the following holds:

1. The project's governance docs (decision log / architecture rules / contributing guide) explicitly say a decision of this category must be cross-validated.
2. The agent honestly believes "I do not have a clearly best answer" — every candidate has comparable downsides, or the problem is outside the agent's training-time confidence.
3. The decision is a **spec-lock** decision: it commits the project to long-term API / ABI / schema / security / capability boundaries that are expensive to revert later.

**Do NOT trigger for**:

- Implementation details (variable naming, helper extraction, log wording, comment polish).
- Decisions the user has already made and just asked the agent to execute.
- Cases where the user explicitly said "skip a2a" / "don't ask other models" / "just give me the candidates".

If unsure whether something is spec-lock or implementation detail, **prefer to consult** rather than self-decide. Better to err on the side of caution.

## Required execution flow

Once triggered, the agent **must** follow these steps in order. Skipping any step is a hard violation.

1. **Announce in chat (before launching anything)**:
   > "This question triggers a2a consultation. Models: <alias-list>. Reason: <one short sentence>. I will pause implementation until the consultation completes."

   The model alias list comes from `a2a models list` — pick from aliases
   the project has actually enabled (the `*` rows are the configured
   default chain). Do not invent alias names.

   The user retains a veto. If the user replies "skip a2a" or similar, switch
   to direct `AskQuestion` and abandon consultation for this turn.

2. **Write a prompt file** to `consultations/<YYYYMMDD-HHMM>-<topic-slug>.prompt.md` using the template at `.cursor/templates/a2a-prompt-template.md` (installed by `a2a init`). The frontmatter must declare:
   - `topic: <slug matching directory name>`
   - `context_files: [...]` — see the next section for the **mandatory** content of this list.

### `context_files:` — what to include (HARD RULE)

`a2a` does **not** auto-attach any project files for you. Whatever the
consulted models can `Read` / `grep` from the workspace mirror is
*exactly* the files you list here, and nothing else. Forgetting a
governance doc is the single most common failure mode — the consulted
models will then propose recommendations that violate red lines they
were never shown, and the synthesis is wasted.

Include, in this order, before the question-specific files:

1. **Governance documents** — every document the project considers
   "must respect": `SPEC.md` / `AGENTS.md` / `CLAUDE.md` / project
   decision logs / ADRs / architecture rules / contributing guide.
   If you can name a "this project's red lines live here" file, it
   belongs in `context_files` of every single a2a consultation in
   that project.
2. **Project conventions / coding rules** — `.cursor/rules/` (the
   whole directory; readonly_mirror walks it recursively),
   `CONTRIBUTING.md`, style guides.
3. **The spec / schema / source files directly under question** —
   what the model needs to *see* to give a meaningful answer.
4. **Closely related code / config** — adjacent modules, the
   immediate caller / callee of a function under question, etc.

Pre-flight checklist before saving the prompt file:

- [ ] Did I include every "must respect" governance doc in the project?
- [ ] Did I include `.cursor/rules/` (or whichever convention dir
      this project uses)?
- [ ] Did I include the actual spec / schema / source under question?
- [ ] If a doc is huge but only one section is relevant, did I
      consider whether to include the whole file (preferred — lets
      the model see surrounding context) vs a smaller subset?
- [ ] Are paths project-relative? (Absolute paths are rejected by
      `a2a`'s readonly_mirror builder with a `tracing::warn!`.)

3. **Invoke a2a**:
   ```
   a2a ask <topic-slug> --prompt-file <path> [--models <list>]
   ```
   This blocks until all models finish. Each raw answer is persisted to `consultations/<YYYYMMDD-HHMM>-<topic-slug>/<model>.answer.md`. A `meta.toml` records which profile was actually used (including any fallback path triggered).

4. **Read every raw answer file**. Do not skim. Note model-by-model:
   - Which candidate they preferred
   - What additional candidates they proposed (if any)
   - What concerns / risks they raised that the agent had not surfaced

5. **Write a synthesis** in the chat. The synthesis MUST contain:
   - **Agreement points**: where do all consulted models agree? Cite each by name.
   - **Disagreement points**: itemize each disagreement, naming which model held which position, with one-sentence reasoning per side. **Never collapse disagreements into a single "the models agreed broadly" sentence.**
   - **New candidates**: if any model proposed an option the agent had not considered, surface it explicitly.
   - **Agent recommendation**: which option, plus 3-5 reasoning points, plus the trade-off the user accepts by choosing it.
   - **Reference**: print the absolute path to `consultations/<dir>/` so the user can open the raw answers themselves.

6. **Ask the user to decide** via `AskQuestion`. The candidates passed to `AskQuestion` should reflect the consultation: at minimum the agent's recommendation plus any meaningful alternative that came out of the multi-model synthesis. **Never skip this step.** The agent's synthesis is input to the user, not a final decision.

7. **After the user picks**, follow the project's normal "record this decision" flow (decision log entry, ADR, etc., depending on the project).

## Anti-patterns (forbidden)

- **Cherry-picking the model that agreed with the agent's prior recommendation** without surfacing dissent.
- **Synthesizing without re-reading raw answers** ("the consulted models probably agreed, let me write the synthesis from memory").
- **Skipping AskQuestion** because "the synthesis is clear enough".
- **Triggering a2a for implementation details** to look thorough.
- **Continuing to implement** while consultation is in flight.
- **Silently falling back** to a different account when the configured one fails — `a2a` already prints fallback transitions; do NOT hide them in the synthesis.
- **Omitting the raw-answers path** from the synthesis — the user must always be one click away from the original model output.
- **Submitting a prompt without governance docs in `context_files`** — the consulted models will hallucinate red lines they cannot see. The pre-flight checklist above is a hard rule, not a suggestion.

## When in doubt

If you are unsure whether a particular question warrants a2a consultation,
ask the user with one short message: "This feels like a spec-lock decision —
should I consult a2a, or do you want to decide directly?" — and let them
choose. That single question is far cheaper than either silently
mis-classifying or burning quota for a trivial decision.
