// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! Prompt frontmatter parsing (YAML frontmatter + markdown body).

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Frontmatter {
    #[serde(default)]
    pub context_files: Vec<String>,
}

pub fn parse_frontmatter(path: &Path) -> Result<Frontmatter> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    parse_frontmatter_str(&raw)
}

pub fn parse_frontmatter_str(raw: &str) -> Result<Frontmatter> {
    let trimmed = raw.trim_start_matches('\u{feff}');
    // Tighter detection: `---` must be followed by a newline to count
    // as the start of YAML frontmatter. A markdown chapter-divider like
    // `--- chapter 1 ---` would otherwise be misread as frontmatter
    // and trigger "frontmatter not closed".
    let has_frontmatter =
        trimmed.starts_with("---\n") || trimmed.starts_with("---\r\n") || trimmed == "---";
    if !has_frontmatter {
        return Ok(Frontmatter::default());
    }
    let after = &trimmed[3..];
    let end = after
        .find("\n---")
        .ok_or_else(|| anyhow!("frontmatter not closed (missing trailing `---`)"))?;
    let yaml = &after[..end];
    parse_minimal_frontmatter_yaml(yaml)
}

fn parse_minimal_frontmatter_yaml(yaml: &str) -> Result<Frontmatter> {
    let mut fm = Frontmatter::default();
    let mut i = 0usize;
    let lines: Vec<&str> = yaml.lines().collect();
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        let (k, v) = match trimmed.split_once(':') {
            Some(kv) => kv,
            None => {
                i += 1;
                continue;
            }
        };
        let k = k.trim();
        let v = v.trim();
        match k {
            // `topic` and `needs_code_execution` are accepted for
            // backwards-compat with existing prompts but ignored —
            // `topic` comes from the CLI positional arg, and
            // `needs_code_execution` was a scratch-mode trigger
            // (SPEC §2.2: scratch deleted).
            "topic" | "needs_code_execution" => {}
            "context_files" => {
                if v.is_empty() || v.starts_with('[') {
                    if v.starts_with('[') && v.ends_with(']') {
                        for item in v[1..v.len() - 1].split(',') {
                            let it = unquote(item.trim());
                            if !it.is_empty() {
                                fm.context_files.push(it.to_string());
                            }
                        }
                    } else {
                        // multi-line list expected; consume `- foo` lines below
                        let mut j = i + 1;
                        while j < lines.len() {
                            let next = lines[j];
                            let nt = next.trim_start();
                            if let Some(rest) = nt.strip_prefix("- ") {
                                let item = unquote(rest.trim());
                                fm.context_files.push(item.to_string());
                                j += 1;
                            } else if nt.is_empty() {
                                j += 1;
                            } else {
                                break;
                            }
                        }
                        i = j;
                        continue;
                    }
                } else {
                    fm.context_files.push(unquote(v).to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    Ok(fm)
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    if ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
        && s.len() >= 2
    {
        return &s[1..s.len() - 1];
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let raw = r#"---
topic: error-classify
needs_code_execution: false
context_files:
  - docs/decision-log.md
  - src/main.rs
---

# Question
body...
"#;
        let fm = parse_frontmatter_str(raw).unwrap();
        // `needs_code_execution` is silently accepted but ignored.
        assert_eq!(fm.context_files.len(), 2);
        assert_eq!(fm.context_files[0], "docs/decision-log.md");
    }

    #[test]
    fn parse_inline_list() {
        let raw = r#"---
topic: foo
context_files: [a.rs, "b.md"]
---
"#;
        let fm = parse_frontmatter_str(raw).unwrap();
        assert_eq!(fm.context_files, vec!["a.rs", "b.md"]);
    }

    #[test]
    fn no_frontmatter() {
        let raw = "# Just markdown\n\nbody\n";
        let fm = parse_frontmatter_str(raw).unwrap();
        assert!(fm.context_files.is_empty());
    }

    #[test]
    fn rejects_chapter_divider_as_frontmatter() {
        // `--- chapter 1 ---` is a markdown chapter divider, not
        // frontmatter. The previous loose `starts_with("---")` check
        // misread it and bailed with "frontmatter not closed".
        let raw = "--- chapter 1 ---\n\nfirst paragraph";
        let fm = parse_frontmatter_str(raw).unwrap();
        assert!(fm.context_files.is_empty());
    }

    #[test]
    fn rejects_dashes_with_other_text() {
        let raw = "---abc\nbody";
        let fm = parse_frontmatter_str(raw).unwrap();
        assert!(fm.context_files.is_empty());
    }
}
