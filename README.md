# a2a (Agent to Agent) 
# — Let your agent reference multiple models' answers in parallel / 让你的 Agent 同时参考多个模型的回答

📖 详细使用说明 / Detailed guide: [English](GUIDE.en_us.md) · [中文](GUIDE.zh_cn.md)

---

Now, within cursor, you can enable an agent to learn about the solutions or viewpoints of other agents regarding a specific problem or matter, and then compile them. This implies that your agent now has its own "think tank".

*现在，你可以在cursor里，让一个Agent对某个问题或事情，了解其它Agent的解决方案或观点，并将其汇总。这意味着，你的Agent有了自己的“智囊团”。*

`a2a` is a Windows command-line tool written in Rust. It allows an Agent (or a human) to simultaneously query **multiple Cursor large language models** - Opus, GPT-5, Gemini, and any other models authorized for use by the Cursor account - through a single prompt, and then save each original answer for comparative review. a2a does not integrate the answers for you; it is merely a tool for "distributing questions and collecting answers".

*`a2a` 是一个用 Rust 编写的Windows命令行工具，它允许 Agent（或人类）通过单一提示并行咨询**多个 Cursor 大型语言模型**——Opus、GPT-5、Gemini 以及 Cursor 账户有权使用的任何其他模型——然后保存每个原始答案以供对比审查。a2a 不会替你整合答案，它只是一个"分发问题、回收答案"的工具。*

## Features / 特性

[English]
- **One prompt → N models** — run concurrently via the `cursor-agent` CLI.
- **Per-call profile chain** (`--profiles a,b,c`) — account-level failures (401 / billing / quota) auto-advance to the next profile, with the dead profile deleted in-flight.
- **Self-contained binary** — profile credentials and model aliases live in a single bundled SQLite file (no `*.toml` configuration); the Cursor skill / rule / prompt templates are baked into the binary via `include_str!`. Distribute one `a2a.exe` on Windows or one `a2a` on Unix — that is all.
- **Raw-answer audit trail** — every consultation persists each model's markdown answer plus a `meta.toml` (profile used, session_id, fallback chain, optional char budget) under `consultations/<timestamp>-<topic>-<uuid>/`.
- **Cursor IDE integration** — `a2a init` installs three skills + one rule + one prompt template that teach the main agent *when* to trigger consultation, *how* to format prompts, *how* to synthesize multi-model answers, and how to drive first-time setup (the user types `a2a_guide` in a fresh Cursor chat).
- **One-shot install wizard** — double-click `a2a.exe` once and it adds itself to user PATH, detects the Cursor CLI, and points the user at the agent-driven setup flow. No installer package needed.

[中文]
- **一个提示 → N 个模型**——通过 `cursor-agent` 命令行并行运行。
- **每次调用的 profile 链**（`--profiles a,b,c`）——账户级故障（401 / 计费 / 配额）会自动跳至下一个 profile，并把失效那个就地删除。
- **自包含的二进制**——profile 凭据和模型别名都存在一个内置的 SQLite 文件里（无需 `*.toml` 配置）；Cursor skill / 规则 / prompt 模板通过 `include_str!` 编进二进制。Windows 分发一个 `a2a.exe`、Unix 分发一个 `a2a`，仅此而已。
- **原始答案审计追踪**——每次咨询都会在 `consultations/<timestamp>-<topic>-<uuid>/` 目录下保存每个模型的 markdown 答案，以及一个 `meta.toml`（包含使用的 profile、session_id、fallback chain、可选的字符预算）。
- **Cursor IDE 集成**——`a2a init` 会安装三个 skill + 一个规则 + 一个 prompt 模板，教会主 agent *何时*触发咨询、*如何*格式化提示、*如何*综合多模型答案、以及如何引导首次配置（用户在新的 Cursor 对话中输入 `a2a_guide`）。
- **一键安装向导**——双击一次 `a2a.exe`，它会把自己加入 user PATH、检测 Cursor CLI，并把用户导向 agent 驱动的安装流程。无需安装包。

## Sample run / 运行示例

After the installation and setup are completed, you can use a2a in the Cursor conversation. For example, you can input the prompt: `Conduct a three-model code review for this project`

*安装并设置完成后，你可以在Cursor的对话中使用a2a，例如，你可以输入提示词：`对本项目进行三模型代码审查`*



```text
$ a2a ask cache-design --prompt-file prompts/cache-design.md \
        --models opus,gpt5,gemini --profiles personal,team

Topic:      cache-design
Models:     ["opus", "gpt5", "gemini"]
Mode:       (per-alias default_mode)
Prompt:     prompts/cache-design.md
Output dir: D:\my-project\consultations\20260430-225014-371-cache-design-d0a8b9

[opus]   profile=personal → calling cursor-agent (phase=fresh)
[gpt5]   profile=personal → calling cursor-agent (phase=fresh)
[gemini] profile=personal → calling cursor-agent (phase=fresh)
[gpt5]   received first response (streaming...)
[gpt5]   still receiving... +210 chars (total 210 chars in last 10s)
[opus]   still alive (no new streamed text in 30s; cursor-agent likely thinking / tool-calling)
[gemini] received first response (streaming...)
[gemini] profile=personal → OK (148.0s)
[gpt5]   profile=personal → OK (279.2s)
[opus]   profile=personal → OK (704.7s)
[opus]   ok
[gpt5]   ok
[gemini] ok

Done. 3 succeeded / 0 failed.
Inspect raw answers in: D:\my-project\consultations\20260430-225014-371-cache-design-d0a8b9
```

After the run, that consultation directory contains:

*运行后的目录结构如下：*

```text
opus.answer.md      gpt5.answer.md      gemini.answer.md
prompt.md           meta.toml
```

The Cursor main agent reads the three answer files, synthesizes the agreement / disagreement points, and presents the user with the final pick via an `AskQuestion`. The user decides; a2a's job is done once the raw answers are on disk.

*Cursor 主 agent 会读这三份原始答案、综合一致/分歧点，然后通过 `AskQuestion` 让用户拍板。用户决定，a2a 只负责把每份原始答案存储为文件。*

## License / 许可

MIT OR Apache-2.0
