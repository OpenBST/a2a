# a2a — 使用说明（中文）

> 🌐 其他语言：[English](GUIDE.en_us.md) · [回到 README](README.md)

这是 a2a 的详细使用说明。如果只想快速了解 a2a 是什么，看 [README.md](README.md) 即可。

## 目录

1. [安装](#1-安装)
2. [首次配置（推荐：交给 Cursor 主对话驱动）](#2-首次配置推荐交给-cursor-主对话驱动)
3. [手动 CLI 配置（备选）](#3-手动-cli-配置备选)
4. [日常使用](#4-日常使用)
5. [CLI 参考](#5-cli-参考)
6. [多账号 fallback](#6-多账号-fallback)
7. [无参数 / agent 入口](#7-无参数--agent-入口)
8. [常见问题排查](#8-常见问题排查)
9. [a2a 怎么存数据](#9-a2a-怎么存数据)

## 1. 安装

### 前置依赖

- **Cursor CLI**（`cursor-agent`）。a2a 是它的封装，必须**单独**装好。下载：<https://cursor.com/cli>；终端验证：`cursor-agent --version`。
- **（仅自行编译需要）** Rust 1.85+：<https://rustup.rs/>。

### 方式 A — 用预编译二进制（推荐）

1. 把 `a2a.exe`（Windows）或 `a2a`（macOS/Linux）放到一个稳定路径。建议：Windows 放 `D:\tools\a2a\a2a.exe`，macOS / Linux 放 `~/.local/bin/a2a`。
2. **双击** `a2a.exe`（Windows），或终端运行 `./a2a`（Unix）。第一次跑会触发欢迎向导：检测它所在目录是否在你的 **user-level PATH** 上；不在的话用 `[Y/n]` 询问后帮你加进去（Windows：用 PowerShell 写 `HKCU\Environment\Path`；Unix：打印 `export PATH=...` 让你自己粘到 shell rc）；之后探测 `cursor-agent`，告诉你装没装好（没装就指向官方安装链接）。
3. **关掉当前终端，重开一个新的。** PATH 修改只对**新启动**的进程生效。
4. 在任意目录下：`a2a --version` 应该能跑通。

### 方式 B — 自行编译

```bash
git clone <仓库地址>
cd a2a
cargo build --release
```

产物：`target/release/a2a.exe`（Windows）/ `target/release/a2a`（Unix）。Windows 项目里附带了 `build.ps1`：每次构建后会把产物复制到 `D:\tools\a2a\a2a.exe`，方便你把 `D:\tools\a2a` 而不是 cargo 的 `target/` 目录放到 PATH 上。

## 2. 首次配置（推荐：交给 Cursor 主对话驱动）

a2a 装好 + Cursor CLI 装好之后，剩下的初始化不用你记任何 CLI 参数，全部由 Cursor 的**主 agent** 完成。

1. 在 Cursor 里打开你的项目。开一个新对话。

2. 把下面任一提示词粘贴并发送：

   英文：
   > Run `a2a --agent` in the terminal and follow its output.

   中文：
   > 在终端运行 `a2a --agent`，根据它的输出执行下一步。

3. 主 agent 会做以下事情：
   1. 跑 `a2a --agent`，读它的 `[health]` 结构化输出，了解当前状态（项目是否初始化、`cursor-agent` 是否能找到、已注册多少个 profile / alias）。
   2. 跑 `a2a init --path <workspace>`，把 3 个 Cursor skill + 1 个规则 + 1 个 prompt 模板装进项目。
   3. 读 `a2a init` 末尾输出的 imperative `[next-step]` 块，把下一步指令告诉**你**。

4. **重启 Cursor。** 关闭所有窗口、重开项目。Cursor 只在启动时加载 `.cursor/skills/` 下的新 skill，所以这步不能省。

5. 开一个**全新的对话**。第一条消息只发这一个单词，**不要**带其它内容：

   ```
   a2a_guide
   ```

   刚装好的 `a2a-setup-guide` skill 会被触发，引导你完成：

   1. 粘贴你的 Cursor API key。agent 会用 stdin 管道（`a2a auth add ... --from-stdin`）传递，确保 token **不**进入聊天日志和 shell history。
   2. 选 1–3 个 model alias 注册（agent 会先跑 `a2a models available` 看你账号能用哪些模型，然后建议常见默认值如 `opus` / `gpt5` / `gemini`）。

6. 完成。从此以后主 agent 会自动用 `a2a-multi-ai-consult` skill（何时该咨询）和 `a2a-operator` skill（自然语言→CLI 翻译）。你正常写代码即可，遇到难决策时它会自动触发 `a2a ask` 并把多个模型的答案综合给你。

## 3. 手动 CLI 配置（备选）

如果不想让 Cursor agent 介入，自己一步步跑也行：

```bash
# 1. 去 https://cursor.com/dashboard → Integrations 申请 API key。
#    a2a 会提示你粘贴（输入隐藏）：
a2a auth add default

# 2. （如果你有多账号）继续注册：
a2a auth add personal
a2a auth add team
a2a auth use default          # 设当前默认 profile

# 3. 看你的 Cursor 账号能用哪些模型：
a2a models available

# 4. 注册一个或多个 alias。**最早注册**的那个 alias 是
#    `a2a ask` 不传 --models 时的默认：
a2a models add opus --model claude-opus-4-7-thinking-xhigh \
    --description "Opus 4.7 1M Thinking Extra High"
a2a models add gpt5 --model gpt-5.5-extra-high \
    --description "GPT-5.5 1M Extra High"
a2a models add gemini --model gemini-3.1-pro \
    --description "Gemini 3.1 Pro"

# 5. （可选）把 Cursor skill + prompt 模板装进某个项目。
#    只用 CLI、不用 Cursor agent 的话可以跳过这步：
cd /path/to/your/project
a2a init
```

验证：`a2a doctor`（人类可读）或 `a2a --agent`（机器可读结构化报告）。

## 4. 日常使用

### 4.1 写一个 prompt 文件

prompt 文件 = markdown 正文 + YAML frontmatter。最简结构：

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

我们的 LRU cache 用 doubly-linked list + HashMap，还是改用
基于 slotmap 的方案？

## Constraints

- 热路径读多写少（读:写 ≈ 20:1）。
- 内存上限：256 MB；每条 entry 约 10 KB；约 25k 条。

## Candidates already considered

### (a) 现状 — DLL + HashMap
...

### (b) 基于 slotmap
...
```

frontmatter 的 `context_files` 列表 = 被咨询模型能"看到"的项目文件。**一定要把项目治理文档放进去**（`SPEC.md` / `AGENTS.md` / `.cursor/rules/` 等）+ 这次问题直接相关的源码。`a2a init` 装的 `a2a-multi-ai-consult` skill 里有完整规则 + 一份 pre-flight 检查表，agent 写 prompt 时会照着来。

`.cursor/templates/a2a-prompt-template.md`（`a2a init` 装的）是个可以直接复制的模板。

### 4.2 跑咨询

```bash
a2a ask <topic-slug> --prompt-file <prompt 文件路径>
```

不传 `--models` 时，a2a 只跑**最早注册**那个 alias。要同时跑多个：

```bash
a2a ask cache-design \
    --prompt-file consultations/2026-04-30-cache.prompt.md \
    --models opus,gpt5,gemini
```

CLI 在模型流式产出过程中会实时打 per-alias 进度行：

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

### 4.3 看结果

每次咨询在 `<project>/consultations/` 下建一个目录：

```
consultations/20260430-225014-371-cache-design-d0a8b9/
├── prompt.md             # 实际发出去的 prompt 拷贝
├── opus.answer.md        # Opus 的原始 markdown 答案
├── gpt5.answer.md
├── gemini.answer.md
└── meta.toml             # 用了哪个 profile / session_id / fallback chain / 耗时
```

如果是从 Cursor 对话里跑的 `a2a ask`，主 agent 会读每份 `<alias>.answer.md`、综合（一致点 / 分歧点 / 新候选），用 `AskQuestion` 让你决定。原始 `*.answer.md` 文件你随时可以自己打开核验。

`meta.toml` 是结构化的：

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

要继续某个模型那次会话，可以用 `cursor-agent --resume <session_id>` 手动接力。

## 5. CLI 参考

### `a2a`（不带任何子命令）

人类用的欢迎向导。检查 PATH、检查 cursor-agent、打印 quick-start。stdin 是 TTY 时**最后会停在 `Press Enter to exit...`**，避免 Windows 下双击窗口闪退。

### `a2a --agent`

同样的状态检查，但是**给 AI agent 的**：永远不停顿、不弹 Y/n，输出是结构化 `[health]` 块（`key: value` 行，可被正则解析）+ 一段 imperative 英文 `[next-step]` 块。Cursor 主 agent 用这个发现系统状态、决定下一步该告诉用户什么。

### `a2a init [--path <project>] [--force]`

把内嵌的 Cursor 模板装进项目。每份模板写两个位置：`<project>/.a2a/template/<rel>`（审计副本，每次都覆盖）和 `<project>/<dst_rel>`（live 副本，在 `.cursor/...` 下；尊重 `--force`）。

### `a2a ask <topic> --prompt-file <path> [flags]`

发起咨询。常用 flag：

| flag | 默认 | 含义 |
|---|---|---|
| `--models a,b,c` | 最早注册的 alias | 要咨询的 alias 列表（逗号分隔）。 |
| `--profiles a,b,c` | 解析后的默认 profile | 本次的 profile 链。账号级失败（KeyDead）→ 删头部、试下一个；网络抖动 → 同 profile 内重试。 |
| `--mode agent\|plan` | per-alias 的 `default_mode` | cursor-agent `--mode` 透传。 |
| `--sandbox enabled\|disabled` | （cursor-agent 默认值） | 透传。 |
| `--no-readonly-prefix` | off | 跳过 readonly 指令注入。 |
| `--dry-run` | off | 打印 cursor-agent 命令，不真跑。 |
| `--budget-only` | off | 估算 char 数，不真跑。 |
| `--log-budget` | off | 给 `meta.toml` 附 `[models.budget]` 表。 |

### `a2a auth ...`

```
a2a auth add <name> [--from-stdin] [--note <text>]
a2a auth list
a2a auth use <name>
a2a auth show <name>           # 掩码：前 4 + 后 4 字符
a2a auth remove <name> [--yes]
a2a auth update <name> [--from-stdin]
```

`--from-stdin` 从 stdin 第一行非空内容读 API key（自动剥 UTF-8 BOM）。任何 agent 驱动 / 脚本场景都强烈推荐用这个 —— token 不会进 shell history。

### `a2a models ...`

```
a2a models                                  # = list（默认）
a2a models list [--verbose]
a2a models available [--profile <name>]
a2a models add <alias> --model <cursor-id> \
    [--mode plan|agent] [--thinking-hint X] \
    [--description X] [--force]
a2a models set <alias> [--model X] [--mode X] \
    [--thinking-hint X] [--description X]
a2a models remove <alias> [--yes]
```

`add --force` 重新定义已存在的 alias 时**保留原 `created_at`**，所以 alias 轮换不会打乱"最早注册=默认"这个语义。

### `a2a doctor` / `a2a status`

`doctor` 检查：a2a 版本 + OS / 架构 + cursor-agent 可达性 + 已注册 profile / alias 数量 + 项目是否 init 过。`status` 是精简版，重点放在 cursor-agent 登录态。

### `a2a list` / `a2a clean`

```
a2a list                                # 当前项目过往的 consultations
a2a clean [--older-than 30d] [--yes]    # 清旧（默认交互确认）
```

每次 `a2a ask` 启动时还会触发一个分离的后台线程，best-effort 地按 7 天清理同项目里旧的 consultation 目录。

### `a2a reset ...`

```
a2a reset models [--yes]         # 清空 SQLite model_aliases 表
a2a reset credentials [--yes]    # 整库删除 ~/.a2a/credentials.db
```

两个都不可逆；不带 `--yes` 时会交互确认。

## 6. 多账号 fallback

a2a 支持把一次咨询跑在一个 profile 链上（典型：主账号 + 1–2 个备账号）。链头如果撞到**账号级失败**（`401 Unauthorized` / billing required / quota exceeded / subscription expired …）：

1. a2a 把链头那个 profile 从 `~/.a2a/credentials.db` **删除**。
2. 把链推进到下一个 profile。
3. 实时打转移信息（`[<alias>] profile=X → KeyDead detected; deleting; advancing fallback chain`）。
4. 每次尝试都进 `meta.toml` 的 `fallback_attempts`。

**网络层失败**（TLS handshake、DNS、`429 rate-limit`、`timeout`）属于**瞬时**错误：在同一个 profile 上重试，最多 3 次，间隔 1s / 3s / 10s。重试时用 `cursor-agent --resume <session_id>` 接续 Cursor 后端的同一会话，所以重试**不会**重发完整 prompt。

```bash
# 先试 personal；它挂了就试 team；再挂就试 default：
a2a ask my-topic --prompt-file <path> --profiles personal,team,default
```

不传 `--profiles` 时是单元素链。默认 profile 解析顺序：

1. SQLite `meta.default_profile`（`a2a auth use` 写入），且对应 profile **仍存在**。
2. 名为 `"default"` 的 profile（如果存在）。
3. 第一个 profile（按 `created_at` 升序，最早注册的那个）。

某个 alias 把链跑完都没成功 → 该 alias 被报失败；其它 alias 继续并行跑。

如果 KeyDead **删掉了凭证库里最后一个 profile**，所有还在跑的 alias 立刻 bail，orchestrator 打恢复指引 banner — 没必要继续对一个空库发 cursor-agent 调用。

## 7. 无参数 / agent 入口

a2a 有两个零子命令的入口，逻辑大同小异，差别在交互性：

| 调用方式 | 触发场景 | 行为 |
|---|---|---|
| `a2a` | Windows 双击 a2a.exe；终端裸跑 `a2a` | PATH 检查 + cursor-agent 检查 + quick-start。需要修 PATH 时弹 `[Y/n]`。stdin 是 TTY 时**末尾停在按回车**，避免控制台闪退。 |
| `a2a --agent` | Cursor 主 agent 在 terminal tool 里调用 | 同样的检查，但**永远不停顿、不弹 Y/n**。输出是结构化 `[health]` 块（`key: value` 行可被正则解析）+ imperative 英文 `[next-step]` 块。 |

两个模式都是**幂等**的：状态完整时再跑只是打一份干净的健康报告就退出，不会改任何东西。

## 8. 常见问题排查

### `cursor-agent NOT in PATH`

a2a 实际调模型靠的是 Cursor CLI。装好它（<https://cursor.com/cli>），重开终端。`a2a doctor` 会确认。

### `a2a --agent` 报 `path_installed: no`

意思是 Cursor agent 用的那个终端 **process 级别 PATH** 不包含当前跑着的 a2a 二进制所在的目录。最常见的原因：Cursor 是在你**修改 user PATH 之前**启动的，所以它的子终端继承的是缓存值。

**修法**：完全关闭所有 Cursor 窗口、重新打开项目。新启动的终端会拉最新注册表 PATH。（`a2a --agent` 检测到这种状态时会自动多打一段"让用户重启 Cursor"的 verbose 提示。）

### `a2a --agent` 报 `stale_a2a_path_entries: N`

user PATH 里还有别的目录也含 `a2a.exe`（典型：`D:\tools\a2a\target\release` 这种 cargo build 产物目录）。在普通终端裸跑一次 `a2a`（不带参数），向导会用 `[Y/n]` 帮你清掉。

### `a2a --agent` 报 `credentials_store: ERROR (...)`

`~/.a2a/credentials.db` 这个 SQLite 文件打不开（损坏 / 被锁 / 权限不对）。`a2a --agent` 的 `[next-step]` 块会走 STOP 路径，避免让 Cursor agent 引导用户去跑 `a2a_guide` —— 后者会调相同的 SQLite 打开逻辑、撞同样的错、形成无诊断的死循环。

去检查那个文件。如果里面没什么舍不得的 profile，直接 `Remove-Item ~/.a2a/credentials.db`（Windows）/ `rm ~/.a2a/credentials.db`（Unix），然后重新 `a2a auth add`。

### `Done. 0 succeeded / N failed.`

每个 alias 都撞了不可恢复的错。看 run 时打的 per-alias `[<alias>] profile=...` 行；每个失败 alias 末尾会附 cursor-agent stderr 的最后 8 行。常见原因：

- 链上每个 profile 的配额都用完了。
- 链上每个 profile 持有的是同一个失效的 Cursor session token。
- 所有要的 model alias 在解析后的 profile 上都不可用（`ModelUnavailable`）。

### 升级 a2a 后想把项目里的 skill 也刷新

```bash
cd /path/to/project
a2a init --force
```

`--force` 用新二进制内嵌的版本覆盖 `.cursor/skills/...`。`<project>/.a2a/template/...` 下的审计副本不论 `--force` 与否**都会**被刷新。**没有** `a2a sync` 子命令——`init --force` 就是那条单步替代路径。

## 9. a2a 怎么存数据

### 唯一的 SQLite 文件

`~/.a2a/credentials.db` 是 a2a 唯一的持久化状态。三个表：

| 表 | 作用 |
|---|---|
| `profiles` | 明文 API key + 几个时间戳。本工具的威胁模型信任本机用户（文件 Unix 0600；Windows 用户级 ACL）。 |
| `meta` | 现在只有 `default_profile`（`a2a auth use` 写入的 profile 名）。 |
| `model_aliases` | user-global 的 model alias 注册表，本机所有 a2a 项目共用同一份。 |

### `a2a init` 会在项目里创建什么

- `<project>/.a2a/` — 项目标记目录（被 `find_project_root` 用到）。
- `<project>/.a2a/template/` — 内嵌模板的审计副本。`a2a init` 每次都覆盖。
- `<project>/consultations/.gitignore` — 忽略 `consultations/` 下所有内容（每次咨询的子目录是用户私有：含原始答案 + meta）。
- `<project>/.cursor/skills/{a2a,a2a-operator,a2a-setup-guide}/SKILL.md` — 三个 Cursor skill。
- `<project>/.cursor/rules/40-a2a-protocol.mdc` — protocol 规则。
- `<project>/.cursor/templates/a2a-prompt-template.md` — 写新 consultation prompt 时复制的模板。

### 哪些参数硬编码（要改就得重 build a2a）：

| 常量 | 值 |
|---|---|
| `PARALLEL` | `true`（多 alias 并行跑） |
| `OUTPUT_ROOT` | `"consultations"`（项目相对路径） |
| `STAGGER_SECS` | `3`（相邻两个 alias spawn 的间隔） |
| `INLINE_PROMPT_MAX_BYTES` | `24_000`（超过即切 indirect prompt） |

### 另请参阅

- [CHANGELOG.md](CHANGELOG.md) — 发布历史。
- [README.md](README.md) — 项目简介。
- `a2a init` 装的三个 skill（在 `<project>/.cursor/skills/` 下）— agent 侧的咨询流程、操作翻译、首次配置向导文档。
