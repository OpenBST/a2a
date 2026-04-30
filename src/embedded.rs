// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! Compile-time bundled templates (SPEC §11).
//!
//! `include_str!` makes `a2a.exe` self-contained: relocating the
//! binary relative to its source tree never breaks template lookup.
//! `cargo run` still works because the same files exist in the
//! source tree at the embed paths.
//!
//! Each template body carries a `{{A2A_VERSION}}` placeholder that
//! [`materialise_template`] substitutes with `crate::A2A_VERSION`
//! at install time, so the "template version" comment in every
//! installed `.cursor/skills/*/SKILL.md` etc. always matches the
//! binary that wrote it.

use crate::A2A_VERSION;

/// Raw template bodies. The `{{A2A_VERSION}}` placeholder is
/// substituted by [`materialise_template`] at install time; callers
/// should never write the raw form directly.
const TEMPLATE_PROMPT: &str =
    include_str!("../templates/prompt-template.md");
const TEMPLATE_RULE_PROTOCOL: &str =
    include_str!("../templates/rules/40-a2a-protocol.mdc");
const TEMPLATE_SKILL_A2A: &str =
    include_str!("../templates/skill/a2a/SKILL.md");
const TEMPLATE_SKILL_OPERATOR: &str =
    include_str!("../templates/skill/a2a-operator/SKILL.md");
const TEMPLATE_SKILL_SETUP_GUIDE: &str =
    include_str!("../templates/skill/a2a-setup-guide/SKILL.md");

/// Manifest entry for a single bundled template file.
///
/// - `stage_rel`: path the **raw** materialised template is staged
///   to under `<project>/.a2a/template/...`. Mirrors the source-tree
///   layout so a user inspecting `<project>/.a2a/template/` sees the
///   familiar `prompt-template.md` / `rules/...` / `skill/.../...`
///   structure. Useful as a per-project audit log of "what a2a init
///   actually wrote on this machine, with which version stamp".
/// - `dst_rel`: path the **installed** copy lives at, relative to
///   the project root. This is the file Cursor actually loads.
#[derive(Debug, Clone, Copy)]
pub struct TemplateAsset {
    pub label: &'static str,
    pub stage_rel: &'static str,
    pub dst_rel: &'static str,
    raw: &'static str,
}

impl TemplateAsset {
    /// Returns the template body with `{{A2A_VERSION}}` replaced by
    /// the current crate version. Always call this — never write
    /// `raw` to disk directly.
    pub fn materialised(&self) -> String {
        materialise_template(self.raw)
    }
}

/// Every template `a2a init` installs into a target project. Each
/// asset is written to TWO locations:
///   1. `<project>/.a2a/template/<stage_rel>` — staged audit copy
///      mirroring the source-tree layout.
///   2. `<project>/<dst_rel>` — the live copy Cursor loads (under
///      `.cursor/skills/...` / `.cursor/rules/...` etc.).
///
/// Both writes happen in `paths::install_templates_into_project`.
pub const TEMPLATE_ASSETS: &[TemplateAsset] = &[
    TemplateAsset {
        label: "prompt template",
        stage_rel: "prompt-template.md",
        dst_rel: ".cursor/templates/a2a-prompt-template.md",
        raw: TEMPLATE_PROMPT,
    },
    TemplateAsset {
        label: "consultation protocol rule",
        stage_rel: "rules/40-a2a-protocol.mdc",
        dst_rel: ".cursor/rules/40-a2a-protocol.mdc",
        raw: TEMPLATE_RULE_PROTOCOL,
    },
    TemplateAsset {
        label: "a2a consultation skill",
        stage_rel: "skill/a2a/SKILL.md",
        dst_rel: ".cursor/skills/a2a/SKILL.md",
        raw: TEMPLATE_SKILL_A2A,
    },
    TemplateAsset {
        label: "a2a operator skill",
        stage_rel: "skill/a2a-operator/SKILL.md",
        dst_rel: ".cursor/skills/a2a-operator/SKILL.md",
        raw: TEMPLATE_SKILL_OPERATOR,
    },
    TemplateAsset {
        label: "a2a setup-guide skill",
        stage_rel: "skill/a2a-setup-guide/SKILL.md",
        dst_rel: ".cursor/skills/a2a-setup-guide/SKILL.md",
        raw: TEMPLATE_SKILL_SETUP_GUIDE,
    },
];

/// Substitute `{{A2A_VERSION}}` with the current crate version. The
/// placeholder is intentionally pure-text (not e.g. `{A2A_VERSION}`
/// or `${VERSION}`) to minimise the chance of collision with anything
/// the templates legitimately want to express literally.
pub fn materialise_template(raw: &str) -> String {
    raw.replace("{{A2A_VERSION}}", A2A_VERSION)
}
