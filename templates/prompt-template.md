---
topic: <slug>
context_files:
  - <relative path>
  - <relative path>
---

<!-- a2a template version: {{A2A_VERSION}} (bundled with the a2a binary; do not hand-edit. Run `a2a init --force` after upgrading a2a to refresh this file.) -->

# AI Design Consultation: {Title}

> Fill in each section. Anything left as `<placeholder>` is a sign the prompt
> isn't ready and the consulted models will produce weaker answers.

## Project context

`<one-line project description: what kind of system, what stage>`

Current phase / milestone: `<...>`

Hard constraints (architectural red lines this question must respect):

- `<constraint 1, e.g. "no dynamic dispatch in hot path">`
- `<constraint 2, e.g. "all I/O must go through capability handles">`

Relevant prior decisions (from this project's decision log / ADRs):

- `<§X.Y or ADR-001 — one-sentence summary>`
- `<§X.Z — one-sentence summary>`

## Problem

`<1-3 short paragraphs: what is the question, why does it need a decision now,
what is the blast radius if we get it wrong>`

## Concrete situation / scenario

If the abstract description above leaves room for ambiguity, sketch a
**minimal scenario** that exposes the trade-off:

```
<example input / call site / data flow>
```

## Candidates already considered

### Candidate (a) `<short name>`

- Design: `<1-2 sentences>`
- Pros: `<...>`
- Cons / trade-off: `<...>`
- Compatibility with constraints above: `<which red lines does it respect / strain>`

### Candidate (b) `<short name>`

- Design:
- Pros:
- Cons:
- Compatibility:

### Candidate (c) `<short name>` (optional)

...

## What we're asking you for

Please return your answer in the following markdown structure so we can
compare it side-by-side with other models:

```
## Recommendation

(a) / (b) / (c) / "your own proposal"

## Reasoning

- <bullet 1>
- <bullet 2>
- <bullet 3>

## Trade-offs accepted by this recommendation

- <risk / cost the user accepts>
- ...

## Better alternative (if any)

If none of the candidates above is good enough, propose your own. Include:
- Design sketch
- How it addresses the trade-offs the listed candidates fail to address
- Expected costs (implementation, runtime, complexity)

## Concerns the question framing missed

If the question itself omits an important consideration (e.g. operational,
security, governance, performance), call it out here so the asker can refine
the prompt.
```

Be specific. Cite the constraints by name where they affect your reasoning.
If a candidate is unworkable for a structural reason (not just preference),
say so plainly.
