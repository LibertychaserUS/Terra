# Terra

**A caged coding agent.** Terra is a small Rust runtime for an LLM agent in which every effect on the world goes through one checkpoint, the rules that decide what may happen are predicates the agent cannot talk to, and the record of what happened cannot be edited afterwards.

[中文说明](#中文说明) · [Caging Engineering](#caging-engineering) · [Quick start](#quick-start) · [Commands](#commands) · [Architecture](#architecture) · [Limits](#limits)

[![CI](https://github.com/LibertychaserUS/Terra/actions/workflows/ci.yml/badge.svg)](https://github.com/LibertychaserUS/Terra/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

---

## What Terra does

Terra runs a standard tool loop (messages in, tool calls out, native function calling against any OpenAI-compatible endpoint) with four things most loops do not have:

| | Mechanism | Where |
|---|---|---|
| **One checkpoint** | Every file, shell, memory and model-call command passes through `CommandExecutor::execute`: audit before, cages, execute, audit after. The loop and the MCP server share it. | `crates/terra-executor` |
| **Cages, not instructions** | Five predicates run in order on each command; the first `Block` stops it. They never rewrite an action. **Specification**: in Goal Mode nothing in the project changes while the goal's spec has open questions. **Memory**: ownership and level permissions. **Meta**: the constitution cannot be written or deleted. **Action**: writes and deletes resolve inside the project. **Verification**: a completion claim needs a verifier run the agent did not make itself. | `crates/terra-caging` |
| **OS-level isolation for shell** | Shell commands run under `sandbox-exec` (macOS) or `bwrap` (Linux): writes outside the project and to protected paths fail at the kernel, not at a string filter. | `crates/terra-executor/src/isolation.rs` |
| **Append-only, hash-chained audit** | Each command and each result is an event whose SHA-256 covers the previous event. `terra trace --verify` walks the chain; an edited log is refused at startup. Declined confirmations are recorded too. | `crates/terra-trace` |

Everything else is an off-the-shelf part: [`async-openai`](https://crates.io/crates/async-openai) for the provider, [`rmcp`](https://crates.io/crates/rmcp) (the official Rust MCP SDK) for the MCP server and client, SQLite with FTS5 for memory and audit, `rustyline` for the prompt.

## Caging Engineering

Terra is an implementation of the [Caging Engineering](https://github.com/LibertychaserUS/caging-engineering) position: an agent is a motion through a state space, and the object of design is which transitions exist, not what the policy is told. Three commitments show up directly in the code.

- **The adjudicator sits outside the agent's writable space.** The agent's own file tool cannot reach the constitution (Meta cage), and its shell cannot either (OS isolation). The audit log is append-only and chained, so a report that was altered is detectable.
- **Specify, then run, then adjudicate.** `/goal` turns the task into a specification first (intent, targets, constraints, success criteria, open questions). While questions are open, the Specification cage blocks every write, delete and shell command. `/done` is a `ClaimComplete` command that the Verification cage refuses unless a `/verify` run passed after the last change.
- **Humans sit at two boundaries.** Only the user changes the rules (the constitution is written by `terra init` and edited by hand) and only the user promotes memory to the project level (`/remember`, `/accept`).

Terra is not a complete reference implementation of the theory; the [Limits](#limits) section says where the mechanisms stop.

## Quick start

Requirements: Rust 1.89+, macOS or Linux, an API key for any OpenAI-compatible endpoint (optional: without one Terra runs on a dry-run stub that echoes your input).

```bash
git clone https://github.com/LibertychaserUS/Terra.git
cd Terra
cargo install --path crates/terra-cli --locked
```

```bash
export DASHSCOPE_API_KEY="..."            # default provider: DashScope, qwen-plus
# or any OpenAI-compatible endpoint:
export TERRA_PROVIDER_API_KEY="..." TERRA_PROVIDER_BASE_URL="https://api.openai.com/v1" TERRA_PROVIDER_MODEL="gpt-4o"

cd your-project
terra init                                # writes .terra/ (constitution, config, prompt, .gitignore)
terra                                     # interactive session
terra run --task "add a greeting to notes.txt" --as-json
terra trace -n 20                         # the audit log; --verify checks the hash chain
terra doctor                              # provider, key source, shell isolation
terra mcp                                 # serve Terra's tools over MCP on stdio
```

A `terra run` task has to name what it changes; an open-ended task is stopped by the Specification cage (exit code 1) unless `require_spec_complete = false` is set in the constitution. Shell and delete calls ask `y/N` in chat; `terra run --yes` and `terra chat --yes` answer yes for you.

## Commands

| Command | What it does |
|---|---|
| `/goal "task"` | Enter Goal Mode. The task is specified first; while the spec has open questions nothing in the project may change. `/goal` again adds detail. |
| `/done` | Claim the goal is complete. Refused until a `/verify` run passed after the last change. On success the model proposes up to three memory candidates. |
| `/accept N` / `/reject N` | Keep a candidate as L1 project memory, or drop it. |
| `/verify [cmd]` | Run the project's test command (or `cmd`) as the verifier. |
| `/abandon` | Leave Goal Mode without a completion claim. |
| `/status` | Mode, goal, verification state, tokens, isolation, tools. |
| `/memory`, `/remember "text"` | List or add L1 project memory. |
| `/trace [n]` | Last n audit entries and the chain head. |
| `/compact`, `/clear` | Summarize older messages with the model, or drop the context. |
| `/commit`, `/undo` | Commit all changes with a model-written message, or discard unstaged edits. Both ask first. |
| `/sessions`, `/help`, `/quit` | |

## The constitution

`.terra/constitution.toml` is the project's rules. The agent cannot write it; you can.

```toml
[meta]
version = 1

[[protected_paths]]          # inside the project, still read-only for the agent
path = ".terra/constitution.toml"
[[protected_paths]]
path = ".terra/audit.db"

[sandbox]
root = "."                   # writes and deletes must resolve under here

[rules]
require_spec_complete = true # Specification cage on
require_verification = true  # Verification cage on
shell_isolation = "auto"     # sandbox-exec / bwrap when available; "none" to turn off

[[mcp_servers]]              # external MCP tool servers, offered as <name>__<tool>
name = "fs"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
```

## Memory

One SQLite table with an FTS5 index. Each entry has an owner, a scope (global, project, session, task) and a level:

| Level | Meaning | Who writes |
|---|---|---|
| L0 | Authoritative | nobody at runtime |
| L1 | Project knowledge | the user (`/remember`, `/accept`) |
| L2 | Task notes | the agent; cleared when the goal ends |
| L3 | Accepted paths | the agent, in Goal Mode |
| L4 | Failed attempts | the agent |
| L5 | Observations | the agent; cleared at session end |
| L6 | Candidates for promotion | the agent; the user decides |

Only the owner of an entry may change or delete it. L1 entries are shown to the model in every system prompt.

## Architecture

```
5  terra-cli        clap commands, readline chat, init/doctor/trace/config
4  terra-agents     the loop, Goal Mode, spec, consolidation, slash commands
3  terra-mcp        MCP server (Terra's tools) and client (external servers), on rmcp
2  terra-executor   audit → cages → execute → audit; tool catalog; shell isolation
1  terra-trace      append-only, hash-chained audit log in SQLite
0  terra-types      command protocol, events, text helpers
   terra-caging     Cage trait, five cages, sandbox paths, constitution
   terra-memory     SQLite + FTS5 memory with ownership and levels
   terra-providers  ProviderAdapter on async-openai; dry-run stub
```

A crate depends only on lower layers; `crates/terra-cli/tests/layering.rs` checks every manifest. About 9,500 lines of Rust including tests.

**One tool call, end to end:** the loop sends the message list and the tool catalog to the model → the model asks for a tool → shell and delete calls ask you `y/N` (a decline is audited) → the executor records the command, runs the cages, executes (shell under OS isolation), records the result → the result goes back to the model as a tool message → the final answer passes the output filter.

## Testing

```bash
cargo test --workspace                                  # no network or API key needed
cargo clippy --workspace --all-targets -- -D warnings
```

The suite covers the cages and path resolution, the executor pipeline, `sandbox-exec` confinement (macOS), the hash chain against tampering, the loop with a scripted provider (roles, tool rounds, declines, gates), the MCP server including sandbox escapes, and the binary in an isolated project.

## Limits

- Shell isolation needs `sandbox-exec` (macOS) or `bwrap` (Linux). Elsewhere, or with `shell_isolation = "none"`, approved shell commands run with your permissions; `terra doctor` and the chat banner say which applies.
- The isolation confines **writes** to the project, the temp directories (`/tmp`, `$TMPDIR`) and the toolchain caches under `$HOME` (`.cargo`, `.npm`, `.cache`, ...). Reads and network access are not restricted.
- External MCP tools are confirmed and audited, but what they do happens in their own process, outside Terra's cages.
- The audit chain detects edits and deletions inside the log; a truncated tail is only visible against a head hash you kept elsewhere (`terra trace --verify` prints it).
- The output filter over model answers is phrase matching.
- `terra mcp` has no confirmation prompt of its own; the MCP client decides whether a shell or delete call runs.

## License

[MIT](LICENSE)

---

<a id="中文说明"></a>
# 中文说明

**Terra 是一个被笼子约束的编码 agent 运行时。** 它用 Rust 写成，核心只有四件事：所有副作用汇到一个检查点；判断"能不能做"的是模型无法说服的谓词；shell 在操作系统层面被隔离；审计日志只追加且带哈希链。

## 它做什么

Terra 跑的是一个标准的工具循环（消息进、工具调用出，对任意 OpenAI 兼容端点用原生 function calling），再加上：

- **单一检查点**：文件、shell、记忆、模型调用，每条命令都经过 `CommandExecutor::execute`，先审计、过笼子、执行、再审计。聊天循环和 MCP 服务端共用它。
- **笼子而不是指令**：五个谓词按序运行，首个 Block 即停。**规格笼**：目标模式下规格还有未解问题时，项目里什么都不能改。**记忆笼**：所有权与等级写权限。**元笼**：宪法文件不可写不可删。**动作笼**：写入与删除必须解析到项目内。**验证笼**：完成声明必须有一次不是 agent 自己做的验证。
- **shell 的 OS 级隔离**：shell 命令在 `sandbox-exec`（macOS）或 `bwrap`（Linux）下运行，写项目外和受保护路径在内核层失败，不靠字符串过滤。
- **只追加、带哈希链的审计**：每条命令和结果是一个事件，SHA-256 覆盖前一个事件。`terra trace --verify` 校验整条链；被改过的日志在启动时被拒绝。用户在确认时拒绝的调用也会记录。

其余部分都是现成件：`async-openai` 做 provider，`rmcp`（官方 Rust MCP SDK）做 MCP 服务端与客户端，SQLite + FTS5 做记忆和审计，`rustyline` 做输入。

## 与 Caging Engineering 的关系

Terra 是 Caging Engineering 立场的一次实现：agent 是状态空间里的一条轨迹，设计对象是"哪些转移存在"，而不是"告诉策略什么"。三条承诺直接体现在代码里：裁决者在 agent 可写空间之外（元笼 + OS 隔离 + 链式审计）；先规格、再运行、再裁决（`/goal` → 规格笼 → `/done` 经验证笼）；人站在两个边界上（只有用户改宪法，只有用户把记忆提升到项目层）。Terra 不是这套理论的完整参考实现，下面的「限制」一节说明了机制在哪里止步。

## 快速开始

```bash
cargo install --path crates/terra-cli --locked
export DASHSCOPE_API_KEY="..."     # 默认 provider：DashScope qwen-plus；也可用 TERRA_PROVIDER_* 指向任意 OpenAI 兼容端点
cd your-project
terra init                         # 生成 .terra/（宪法、配置、提示词、.gitignore）
terra                              # 交互会话
terra run --task "给 notes.txt 加一句问候" --as-json
terra trace -n 20                  # 审计日志；--verify 校验哈希链
terra doctor                       # provider、key 来源、shell 隔离方式
terra mcp                          # 以 MCP 服务端形式提供 Terra 的工具
```

`terra run` 的任务要写明改什么；开放式任务会被规格笼拦下（退出码 1），除非宪法里设 `require_spec_complete = false`。聊天里 shell 和删除会询问 y/N；`--yes` 自动同意。

## 命令

| 命令 | 作用 |
|---|---|
| `/goal "任务"` | 进入目标模式。任务先被规格化；规格有未解问题时项目里什么都不能改。再次 `/goal` 补充细节。 |
| `/done` | 声明目标完成。最后一次改动之后必须有通过的 `/verify`，否则被拒。成功后模型提出至多三条记忆候选。 |
| `/accept N` / `/reject N` | 把候选保留为 L1 项目记忆，或丢弃。 |
| `/verify [命令]` | 以验证者身份运行项目的测试命令（或指定命令）。 |
| `/abandon` | 不声明完成，退出目标模式。 |
| `/status` `/memory` `/remember` `/trace` `/compact` `/clear` `/commit` `/undo` `/sessions` `/help` `/quit` | 见 `/help`。 |

## 限制

- shell 隔离依赖 `sandbox-exec`（macOS）或 `bwrap`（Linux）；没有它们或设了 `shell_isolation = "none"` 时，已批准的 shell 命令以你的权限运行，`terra doctor` 和聊天横幅会说明当前是哪种。
- 隔离约束的是**写入**：允许项目目录、临时目录（`/tmp`、`$TMPDIR`）和 `$HOME` 下的工具链缓存（`.cargo`、`.npm`、`.cache` 等）；读取和网络不受限。
- 外部 MCP 工具会被确认和审计，但它们做的事发生在自己的进程里，在 Terra 的笼子之外。
- 审计链能发现日志内部的篡改和删除；末尾截断只有对照你另外保存的链头哈希才能发现。
- 对模型回答的输出过滤是短语匹配。

## 许可证

[MIT](LICENSE)
