// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! `a2a` (no subcommand) and `a2a --agent`: welcome / health entry
//! points.
//!
//! Two modes share most of the logic but differ in interactivity:
//!
//! - **`a2a` (no args)** — human welcome wizard, triggered when a
//!   user double-clicks `a2a.exe` from Explorer or types `a2a` in
//!   a terminal with zero arguments. Three steps:
//!     1. Is the dir containing `a2a.exe` on the user-level PATH?
//!        If not, offer Y/n to append it (HKCU\Environment\PATH on
//!        Windows via PowerShell shellout; on Unix print the
//!        rc-file recipe and skip the in-process modification,
//!        which would otherwise have to guess between bash / zsh /
//!        fish / etc.).
//!     2. Is `cursor-agent` reachable via `locate_binary()`?
//!     3. Print quick-start hints.
//!
//!   On Windows GUI launches the console snaps shut after exit, so
//!   if stdin is a TTY we pause for Enter at the end.
//!
//! - **`a2a --agent`** — same situational checks but tailored for
//!   AI-agent consumption: zero pause, zero Y/n prompts, structured
//!   health report + imperative next-step guidance the agent can
//!   echo to the user. Cursor's terminal tool runs the spawned
//!   process with a TTY stdin, so a Y/n prompt or pause-for-Enter
//!   would deadlock the agent — `--agent` mode short-circuits both.
//!
//! Both modes are idempotent: re-running after everything is already
//! configured prints a clean health report and exits without
//! offering to modify state.

use anyhow::{Context, Result};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

pub fn run() -> Result<()> {
    print_banner();

    let exe_dir = current_exe_dir()?;
    println!();
    println!("[1/3] PATH check");
    let analysis = analyze_user_path(&exe_dir);
    let stale = &analysis.stale_a2a_entries;
    match (analysis.current_dir_present, stale.is_empty()) {
        (true, true) => {
            println!("  [ok] {} is on your user PATH.", exe_dir.display());
        }
        (true, false) => {
            println!("  [ok] {} is on your user PATH.", exe_dir.display());
            println!(
                "  [!!] But {} OTHER PATH entry(ies) also contain an a2a.exe (likely",
                stale.len()
            );
            println!("       stale — leftover from a previous install location):");
            for s in stale {
                println!("         - {s}");
            }
            println!("       Removing them keeps a single canonical a2a binary on PATH so");
            println!("       `a2a` always resolves to the one you just double-clicked.");
            if confirm_y("       Remove the stale entry(ies) now? [Y/n]:") {
                let to_remove: Vec<&str> = stale.iter().map(String::as_str).collect();
                match remove_from_user_path(&to_remove) {
                    Ok(()) => {
                        println!(
                            "  [ok] Removed. Open a NEW terminal for the change to take effect."
                        );
                    }
                    Err(e) => {
                        println!("  [!!] Could not modify user PATH: {e:#}");
                        println!("       You can remove them manually via:");
                        println!("         Win key → \"Edit the system environment variables\"");
                        println!("         → Environment Variables → User variables → Path → Edit");
                    }
                }
            } else {
                println!("  [skip] Stale entries left in place. Re-run a2a (no args) anytime.");
            }
        }
        (false, true) => {
            println!("  [!!] {} is NOT on your user PATH.", exe_dir.display());
            println!("       Adding it lets you type `a2a ...` from any directory");
            println!("       in a new terminal, instead of the full path to a2a.exe.");
            if confirm_y("       Add it now? [Y/n]:") {
                match add_to_user_path(&exe_dir) {
                    Ok(()) => {
                        println!("  [ok] Added. Open a NEW terminal for the change");
                        println!("       to take effect (existing terminals keep their old PATH).");
                    }
                    Err(e) => {
                        println!("  [!!] Could not modify user PATH: {e:#}");
                        println!("       You can add it manually:");
                        println!("         Win key → \"Edit the system environment variables\"");
                        println!("         → Environment Variables → User variables → Path → Edit");
                        println!("         → New → paste {}", exe_dir.display());
                    }
                }
            } else {
                println!("  [skip] You can re-run a2a (no args) anytime to add it later.");
            }
        }
        (false, false) => {
            println!("  [!!] {} is NOT on your user PATH.", exe_dir.display());
            println!(
                "  [!!] {} OTHER PATH entry(ies) also contain an a2a.exe (likely stale):",
                stale.len()
            );
            for s in stale {
                println!("         - {s}");
            }
            println!();
            println!("       Plan:");
            println!("         + add    {}", exe_dir.display());
            for s in stale {
                println!("         - remove {s}");
            }
            if confirm_y("       Apply this plan now? [Y/n]:") {
                let mut errors: Vec<String> = Vec::new();
                if let Err(e) = add_to_user_path(&exe_dir) {
                    errors.push(format!("add  {}: {e:#}", exe_dir.display()));
                }
                let to_remove: Vec<&str> = stale.iter().map(String::as_str).collect();
                if let Err(e) = remove_from_user_path(&to_remove) {
                    errors.push(format!("remove stale: {e:#}"));
                }
                if errors.is_empty() {
                    println!("  [ok] Applied. Open a NEW terminal for the change to take effect.");
                } else {
                    for err in &errors {
                        println!("  [!!] {err}");
                    }
                    println!("       You can adjust user PATH manually:");
                    println!("         Win key → \"Edit the system environment variables\"");
                    println!("         → Environment Variables → User variables → Path");
                }
            } else {
                println!("  [skip] No changes made. Re-run a2a (no args) anytime to apply.");
            }
        }
    }

    println!();
    println!("[2/3] Cursor CLI check");
    match crate::runner::cursor_agent::locate_binary() {
        Some(p) => {
            println!("  [ok] cursor-agent found: {}", p.display());
        }
        None => {
            println!("  [!!] cursor-agent is NOT on your PATH and not in the");
            println!("       canonical install dir. a2a needs it to talk to the");
            println!("       Cursor backend; without it `a2a ask` and");
            println!("       `a2a models available` cannot run.");
            println!();
            println!("       Install Cursor CLI:");
            if cfg!(target_os = "windows") {
                println!("         Open https://cursor.com/cli and run the Windows installer.");
                println!("         Then open a new terminal and re-run a2a (no args) to verify.");
            } else {
                // macOS and Linux share the same one-liner installer.
                println!("         curl -fsSL https://cursor.com/install | sh");
            }
        }
    }

    println!();
    println!("[3/3] Next step / 下一步");
    println!();
    println!("[Recommended way / 推荐方式]");
    println!();
    println!("(Drive setup from Cursor's main agent)");
    println!("（通过 Cursor 主对话完成安装）");
    println!();
    println!("  Open Cursor on your project, open a main-agent chat, and paste");
    println!("  one of the prompts below. The agent will run `a2a --agent` +");
    println!("  `a2a init --path <workspace>` for you, then ask you to restart");
    println!("  Cursor and type `a2a_guide` in a new chat to finish API key +");
    println!("  model alias registration.");
    println!();
    println!("  +----------------------------------------------------------------------+");
    println!("  | English prompt — paste into Cursor:                                  |");
    println!("  | ( ↓↓↓ copy the line below ↓↓↓ )                                        |");
    println!("  +----------------------------------------------------------------------+");
    println!();
    println!("    Run `a2a --agent` in the terminal and follow its output.");
    println!();
    println!("  在 Cursor 中打开项目，开启主对话，粘贴下面任一提示词。Agent 会");
    println!("  替你执行 `a2a --agent` 和 `a2a init --path <workspace>`，然后");
    println!("  让你重启 Cursor、在新对话中输入 `a2a_guide`，完成 API key +");
    println!("  模型别名（model alias）的注册。");
    println!();
    println!("  +----------------------------------------------------------------------+");
    println!("  | 中文提示词 — 粘贴到 Cursor 主对话：                                    |");
    println!("  |（ ↓↓↓ 复制下面一行 ↓↓↓ ）                                             |");
    println!("  +----------------------------------------------------------------------+");
    println!();
    println!("    在终端运行 `a2a --agent`，根据它的输出执行下一步。");
    println!();
    println!("[Manual fallback / 手动方式]");
    println!();
    println!("(if you'd rather type CLI commands yourself)");
    println!("（如果你想自己输 CLI 命令）");
    println!();
    println!("  a2a auth add  ->  a2a models add  ->  a2a ask    (a2a --help for details)");

    pause_if_interactive();
    Ok(())
}

/// `a2a --agent`: structured health report + imperative next-step
/// guidance for an AI agent invoking a2a from Cursor's terminal.
/// Never pauses, never prompts for Y/n. Emits machine-friendly
/// `key: value` lines under a `[health]` block so the agent can
/// regex-parse them if needed, plus an `[next-step]` block worded
/// as direct imperatives the agent can echo to the user.
pub fn run_agent() -> Result<()> {
    println!("================================================================");
    println!("  a2a {} — agent-mode introduction", crate::A2A_VERSION);
    println!("================================================================");
    println!();
    println!("a2a is a CLI bridge that lets an AI agent consult multiple Cursor LLMs");
    println!("(Opus / GPT-5 / Gemini / etc.) in parallel from one prompt and synthesize");
    println!("the answers. This invocation is `--agent` mode — I never pause for human");
    println!("input here, so it's safe to read my stdout in a single shot.");

    let exe_dir = current_exe_dir().ok();
    let path_analysis = exe_dir.as_deref().map(analyze_user_path);
    let path_state = match &path_analysis {
        Some(a) if a.current_dir_present => "yes",
        Some(_) => "no",
        None => "unknown",
    };
    let stale_count = path_analysis
        .as_ref()
        .map(|a| a.stale_a2a_entries.len())
        .unwrap_or(0);
    let cursor_agent = crate::runner::cursor_agent::locate_binary();
    // `find_project_root::is_some()` is the authoritative answer
    // (its doc comment explains why we don't re-derive
    // `r.join(".a2a").is_dir()`).
    let cwd_opt = std::env::current_dir().ok();
    let project_root: Option<std::path::PathBuf> =
        cwd_opt.as_deref().and_then(crate::paths::find_project_root);
    let project_initialised = project_root.is_some();
    // SPEC §3.5.2: surface store-open failures distinctly. Silently
    // collapsing them to "0 profiles / 0 aliases" routes the agent
    // to `a2a_guide`, where the setup-guide skill hits the same
    // store-open error — that's an infinite loop with no signal.
    let store_open_err: Option<String>;
    let n_profiles: usize;
    let n_aliases: usize;
    match crate::auth::store::open() {
        Ok(s) => {
            n_profiles = s.list_profiles().map(|v| v.len()).unwrap_or(0);
            n_aliases = s.list_model_aliases().map(|v| v.len()).unwrap_or(0);
            store_open_err = None;
        }
        Err(e) => {
            n_profiles = 0;
            n_aliases = 0;
            store_open_err = Some(format!("{e:#}"));
        }
    }

    println!();
    println!("[health]");
    println!("  path_installed:       {}", path_state);
    match &cursor_agent {
        Some(p) => println!("  cursor_agent:         found ({})", p.display()),
        None => println!("  cursor_agent:         NOT FOUND"),
    }
    if let Some(err) = &store_open_err {
        println!("  registered_profiles:  unknown   (credentials_store ERROR)");
        println!("  registered_aliases:   unknown   (credentials_store ERROR)");
        println!("  credentials_store:    ERROR ({err})");
    } else {
        println!("  registered_profiles:  {n_profiles}");
        println!("  registered_aliases:   {n_aliases}");
    }
    match &project_root {
        Some(r) => println!(
            "  current_project:      {}  (initialised: {})",
            r.display(),
            if project_initialised { "yes" } else { "no" }
        ),
        None => match &cwd_opt {
            Some(c) => println!("  current_project:      {}  (initialised: no)", c.display()),
            None => println!("  current_project:      <unknown cwd>  (initialised: unknown)"),
        },
    }
    if stale_count > 0
        && let Some(a) = &path_analysis
    {
        println!("  stale_a2a_path_entries: {stale_count}    (cleanable via `a2a` no-args wizard)");
        for s in &a.stale_a2a_entries {
            println!("                          - {s}");
        }
    }

    println!();
    println!("[next-step for you, the agent]");
    if cursor_agent.is_none() {
        println!("  STOP. cursor-agent is missing from PATH and the canonical install dir.");
        println!("  a2a cannot consult any model without it. Tell the user to install");
        println!("  Cursor CLI from https://cursor.com/cli and re-run a2a --agent after");
        println!("  reopening their terminal.");
        return Ok(());
    }
    if let Some(err) = &store_open_err {
        // Same loop-prevention rationale as the [health] block above.
        println!("  STOP. ~/.a2a/credentials.db could not be opened:");
        println!("         {err}");
        println!("  a2a uses this database for both API keys and model aliases, so");
        println!("  no further setup commands can run. Tell the user to:");
        println!("    - inspect the file (it lives in their user home: ~/.a2a/credentials.db).");
        println!("    - if it's corrupt and they have no irreplaceable profiles, delete it");
        println!("      and re-run `a2a auth add` to start fresh.");
        println!("    - otherwise, fix permissions / locate the cause of the IO error.");
        return Ok(());
    }
    if !project_initialised {
        // Suggest the cwd as the workspace root when find_project_root
        // returned None: the agent's terminal is already there, and
        // running `a2a init --path <cwd>` is what the agent should do
        // unless the user explicitly wants a different project root.
        let path_arg = project_root
            .as_ref()
            .map(|r| r.display().to_string())
            .or_else(|| cwd_opt.as_ref().map(|c| c.display().to_string()))
            .unwrap_or_else(|| "<absolute_workspace_path>".to_string());
        println!("  current_project is NOT initialised yet. Do this:");
        println!();
        println!("    1. Run:  a2a init --path {path_arg}");
        println!("    2. Read its output (it ends with another agent-instruction block).");
        println!("    3. Tell the user to:");
        println!("         - close all Cursor windows, reopen this project");
        println!("         - open a NEW chat");
        println!("         - type the single word: a2a_guide");
        println!("       That triggers the freshly-installed `a2a-setup-guide` skill,");
        println!("       which walks them through API key + model alias registration.");
    } else if n_profiles == 0 || n_aliases == 0 {
        println!("  current_project is initialised, but credentials / aliases are missing.");
        println!("  Tell the user to:");
        println!("    - close Cursor, reopen this project");
        println!("    - open a NEW chat");
        println!("    - type the single word: a2a_guide");
        println!("  That triggers the `a2a-setup-guide` skill which walks them through");
        println!("  registering the missing credentials / aliases.");
    } else {
        // Sub-case worth flagging before "all good": current_exe lives
        // outside the user-level PATH but other a2a entries are on
        // PATH. Almost always means Cursor was launched before the
        // user cleaned PATH; this terminal still holds the stale
        // process env, so Windows resolved `a2a` to the pre-cleanup
        // location while the registry-PATH points at the canonical
        // one. Non-blocking (this exe still runs), but the next IDE
        // restart will route `a2a` to a *different file*, which is
        // confusing forensically and should be flagged proactively.
        if let Some(a) = &path_analysis
            && !a.current_dir_present
            && !a.stale_a2a_entries.is_empty()
        {
            println!("  Note: current_exe is NOT on the user-level PATH (registry HKCU).");
            println!("  The running binary likely came from a stale process env held");
            println!("  by the IDE — Cursor was launched before the user's PATH was");
            println!("  cleaned, so this terminal still resolves `a2a` to the old");
            println!("  location. The clean entry currently on PATH is:");
            for s in &a.stale_a2a_entries {
                println!("    - {s}");
            }
            println!("  Tell the user to close all Cursor windows and reopen this");
            println!("  project; new terminals in the fresh session will pick up");
            println!("  the latest user PATH and resolve `a2a` there. Until then,");
            println!("  a2a still works — this is a forensic warning, not a block.");
            println!();
        }
        println!("  Everything looks good. a2a is ready to consult. Suggested next:");
        println!("    - Write a prompt file (see .cursor/templates/a2a-prompt-template.md");
        println!("      for the expected frontmatter).");
        println!("    - Run:  a2a ask <topic-slug> --prompt-file <path-to-prompt.md>");
        println!("  See the `a2a-multi-ai-consult` skill for *when* to consult, and");
        println!("  the `a2a-operator` skill for ongoing natural-language operations.");
    }
    Ok(())
}

fn print_banner() {
    println!("================================================================");
    println!(
        "  a2a {} — Multi-AI consultation bridge",
        crate::A2A_VERSION
    );
    println!("================================================================");
}

fn current_exe_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("could not resolve a2a executable path")?;
    let dir = exe
        .parent()
        .context("a2a executable has no parent directory")?
        .to_path_buf();
    // Strip Windows verbatim prefix (\\?\) so the displayed path is
    // human-friendly. Functionally equivalent.
    let s = dir.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        return Ok(PathBuf::from(stripped));
    }
    Ok(dir)
}

/// Analysis of the user-level PATH with respect to the currently
/// running `a2a.exe`. Combines two questions the wizard needs to
/// answer in one place:
///
///   - Is `current_exe_dir` already on PATH?
///   - Are there *other* PATH entries that contain an `a2a.exe`
///     (i.e. previous install locations the user probably wants to
///     clean up — e.g. `D:\tools\a2a\target\release\` after a
///     relocation to `D:\tools\a2a\`)?
///
/// "Stale" is detected by `<entry>\a2a.exe` actually existing on
/// disk. Dirs that no longer exist (orphaned PATH entries) are
/// intentionally not flagged — those are general OS PATH hygiene,
/// not something a2a should clean autonomously, and many users
/// have legitimate dead PATH entries we don't want to touch.
#[derive(Debug)]
struct UserPathState {
    current_dir_present: bool,
    stale_a2a_entries: Vec<String>,
}

fn analyze_user_path(current_exe_dir: &Path) -> UserPathState {
    let user_path = read_user_path().unwrap_or_default();
    let target_norm = normalize(current_exe_dir.to_string_lossy().as_ref());
    let mut state = UserPathState {
        current_dir_present: false,
        stale_a2a_entries: Vec::new(),
    };
    // `std::env::split_paths` uses the host's PATH separator
    // (`;` on Windows, `:` on Unix); a hard-coded `';'` here would
    // make the Unix wizard never recognise any current entry.
    for entry in std::env::split_paths(&user_path) {
        let s = entry.to_string_lossy();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            continue;
        }
        let e_norm = normalize(trimmed);
        if e_norm == target_norm {
            state.current_dir_present = true;
            continue;
        }
        // Heuristic: "stale" = the directory in PATH contains an
        // a2a binary that isn't the currently-running one. Probe
        // both Windows (`a2a.exe`) and Unix (`a2a`) names so the
        // detection works on whichever host this build runs on.
        if entry.join("a2a.exe").exists() || entry.join("a2a").is_file() {
            state.stale_a2a_entries.push(trimmed.to_string());
        }
    }
    state
}

fn normalize(p: &str) -> String {
    p.trim().trim_end_matches(['\\', '/']).to_lowercase()
}

#[cfg(windows)]
fn read_user_path() -> Result<String> {
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Environment]::GetEnvironmentVariable('PATH', 'User')",
        ])
        .output()
        .context("read user PATH via powershell.exe")?;
    if !output.status.success() {
        anyhow::bail!(
            "powershell exited {} reading user PATH: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(not(windows))]
fn read_user_path() -> Result<String> {
    // No reliable equivalent of HKCU\Environment\PATH on Unix; the
    // user-level PATH is whatever the shell rc files (.bashrc /
    // .zshrc / fish config / etc.) export. Fall back to the
    // currently-effective PATH so the same dir-comparison logic
    // works for the "is `a2a.exe` reachable from this shell?" case.
    Ok(std::env::var("PATH").unwrap_or_default())
}

#[cfg(windows)]
fn add_to_user_path(dir: &Path) -> Result<()> {
    let dir_str = dir.to_string_lossy().to_string();
    // PowerShell single-quote escape: every char literal except `'`
    // which becomes `''`. Avoids interpolation surprises in the
    // dir path (drive letters, spaces, etc.).
    let escaped = dir_str.replace('\'', "''");
    let script = format!(
        r#"$dir = '{escaped}';
$cur = [Environment]::GetEnvironmentVariable('PATH', 'User');
if ($null -eq $cur) {{ $cur = '' }}
# Trim a trailing ';' so the join below can't produce ';;' (ugly
# and brittle for naive Split(';') parsers).
$cur = $cur.TrimEnd(';');
$existing = $cur.Split(';') | Where-Object {{ $_ -ne '' }};
$found = $false;
foreach ($e in $existing) {{
    if ([string]::Equals($e.TrimEnd('\').TrimEnd('/'), $dir.TrimEnd('\').TrimEnd('/'), [System.StringComparison]::OrdinalIgnoreCase)) {{
        $found = $true; break
    }}
}}
if (-not $found) {{
    $new = if ($cur -eq '') {{ $dir }} else {{ "$cur;$dir" }};
    [Environment]::SetEnvironmentVariable('PATH', $new, 'User');
}}
"#
    );
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .context("modify user PATH via powershell.exe")?;
    if !output.status.success() {
        anyhow::bail!(
            "powershell exited {} modifying user PATH: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn add_to_user_path(dir: &Path) -> Result<()> {
    // We don't auto-edit .bashrc / .zshrc / fish config because
    // every shell uses different rc paths and append-vs-prepend
    // conventions, and a wrong edit can lock a user out of their
    // shell. Print the recipe and let them apply it.
    println!();
    println!("       Manual step (Unix): append to your shell rc");
    println!("       (~/.bashrc / ~/.zshrc / ~/.config/fish/config.fish):");
    println!();
    println!("           export PATH=\"{}:$PATH\"", dir.display());
    println!();
    println!("       Then `source` it or open a new terminal.");
    Ok(())
}

/// Remove the listed directories from the user-level PATH. Each
/// entry is matched case-insensitively against `HKCU\Environment\Path`
/// after trimming trailing `\` / `/`. Entries not currently present
/// are silently ignored (the goal is the post-state, not a strict
/// "must remove" contract). Empty input is a no-op.
#[cfg(windows)]
fn remove_from_user_path(entries: &[&str]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    // Build a PowerShell `@('a', 'b', 'c')` literal with each entry
    // single-quoted; escape any inner `'` as `''` per PS rules.
    let escaped: Vec<String> = entries
        .iter()
        .map(|e| format!("'{}'", e.replace('\'', "''")))
        .collect();
    let to_remove = escaped.join(",");
    let script = format!(
        r#"$toRemove = @({to_remove});
$cur = [Environment]::GetEnvironmentVariable('PATH', 'User');
if ($null -eq $cur) {{ exit 0 }}
$kept = $cur.Split(';') | Where-Object {{
    $e = $_.TrimEnd('\').TrimEnd('/');
    if ($e -eq '') {{ return $false }}
    $drop = $false;
    foreach ($r in $toRemove) {{
        if ([string]::Equals($e, $r.TrimEnd('\').TrimEnd('/'), [System.StringComparison]::OrdinalIgnoreCase)) {{
            $drop = $true; break
        }}
    }}
    -not $drop
}};
if ($kept -is [array]) {{
    $new = ($kept -join ';');
}} elseif ($null -eq $kept) {{
    $new = '';
}} else {{
    $new = [string]$kept;
}}
[Environment]::SetEnvironmentVariable('PATH', $new, 'User');
"#
    );
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .context("remove user PATH entries via powershell.exe")?;
    if !output.status.success() {
        anyhow::bail!(
            "powershell exited {} removing user PATH entries: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn remove_from_user_path(entries: &[&str]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    println!();
    println!("       Manual step (Unix): remove these entries from your shell rc");
    println!("       (~/.bashrc / ~/.zshrc / ~/.config/fish/config.fish):");
    for e in entries {
        println!("         {e}");
    }
    println!("       Then `source` it or open a new terminal.");
    Ok(())
}

fn confirm_y(prompt: &str) -> bool {
    use std::io::Write;
    if !std::io::stdin().is_terminal() {
        // Non-interactive (piped / redirected stdin): err on the
        // side of NO state mutation.
        return false;
    }
    print!("{prompt} ");
    let _ = std::io::stdout().flush();
    let mut buf = String::new();
    match std::io::stdin().read_line(&mut buf) {
        Ok(_) => {
            let s = buf.trim().to_lowercase();
            s.is_empty() || s == "y" || s == "yes"
        }
        Err(_) => false,
    }
}

/// On Windows, double-clicking `a2a.exe` opens a console window that
/// snaps shut the moment the process exits — the user never sees the
/// wizard output. Pause for Enter when stdin is a TTY so the
/// double-click case stays readable. Skipped when stdin is piped /
/// redirected (scripts shouldn't hang waiting for nobody to press
/// Enter).
fn pause_if_interactive() {
    use std::io::Write;
    if !std::io::stdin().is_terminal() {
        return;
    }
    println!();
    print!("Press Enter to exit... ");
    let _ = std::io::stdout().flush();
    let mut buf = String::new();
    let _ = std::io::stdin().read_line(&mut buf);
}
