# Thread/Stage 工作状态：目标架构 + 最小可用骨架

> 本文融合 docs 下两份方案：
> - **A** `thread-chat-stage-scalable-stonebraker.md`（实现导向，精确到 file:line，改动面最小）
> - **B** `thread-stage-chat-snapshot-plan.md`（设计导向，完整建模，工程严谨）
>
> 用 **目标架构** 锚定方向（取 B 的完整建模），用 **执行骨架** 锚定本轮最小可用（借 A 的"端到端一条薄线 + 最大复用"思路）。原则：**骨架是目标架构的真子集，绝不骨架一套、终态另一套**。

---

## Context（为什么做这个）

Sessio 里 thread 是工作流容器、stage 是其中阶段，各自关联若干 agent sessions。三个递进需求（用户逐步明确）：
1. **快照**：从 thread/stage 发起新 chat 时，把"那一刻"的整体状况冻结成上下文喂给 agent，回看走冻结快照。
2. **结构化状态**：stage 当前**无任何存储状态**——`threads`/`stages`/`thread_stages` 表无 status 列，ThreadPage 的 done/active/pending 是 `stageState()`（src/pages/ThreadPage.tsx:315）按顺序相对 `thread.stageId` **实时推算**的；"问题/障碍"无处可存。需要显式建模。
3. **CLI 闭环**：sessio **已有成熟 CLI**（src-tauri/src/cli.rs，src-tauri/src/main.rs:5 `args_os().len()>1` 即走 CLI，与 GUI 同一二进制、开同一个 `~/.sessio/db-data/sessio-index.db`、已有 sessions/memory/config 命令族且全支持 `--json`）。扩展 thread/stage 命令族，让正在干活的 agent 自己回写 stage 状态。

## 已确认决策

| 维度 | 选定 | 来源 |
|---|---|---|
| 状态枚举 | **6 种** `not_started/in_progress/blocked/needs_review/completed/skipped` | B |
| 问题建模 | **独立 `thread_stage_issues` 表**（title/severity/status，可统计可 CRUD） | B |
| 快照存储 | **独立 `thread_work_snapshots` 表**（结构化 envelope，可独立查询） | B |
| 节奏 | **目标架构定方向 + 最小骨架先落地** | 融合 |

## 方案 A vs B：优缺点与融合取舍

- **A 优**：复用现有 fork/快照链路（回看 UI 零改动）、0 新表、1 command、风险可控、可立即实现。**A 缺**：`note` 自由文本无法统计；缺 needs_review/skipped；快照是发起时烤进对话附件的死 markdown，事后不可独立查询、无进度 rollup。
- **B 优**：issue 一等公民可按 severity 统计、状态贴合真实工作流（呼应已有 `KanbanStatus`（src-tauri/src/models.rs:386）的 agent_review/human_review）、work-state 是独立可查实体（原始文件丢了也能渲染状况）、测试矩阵完整。**B 缺**：3 表 + 7 command 工作量数倍、停留设计层无 file:line、两套快照并存有一致性成本、**issue 表若无配套 UI 易"建表没人填"**。
- **融合取舍**：终态采 B 的建模（表达力与鲁棒性值得）；但**分阶段落地**，第一刀只切"状态读写 + 编辑 + 一个 CLI 写命令"这条最能立刻产生价值的端到端线（它直接取代顺序推算这一现有行为）。issue/snapshot 表按 codebase 既有的增量迁移习惯（`SCHEMA_V1..V5`）后续追加，**架构已为它们预留位置**，骨架不会被推翻。

---

# 目标架构（方向锚定 · 终态）

> 所有实现不得违背本节。后续每个阶段只是往这个骨骼上填肉。

## 数据模型（三张新表，增量迁移）

```sql
-- 取代"顺序推算"。独立表 → 支持 lazy 缺省（无行时读侧现算，写时才落库）
thread_stage_states(
  thread_stage_id TEXT PRIMARY KEY,   -- 对应 thread_stages.id
  status   TEXT NOT NULL,             -- 6 态之一
  summary  TEXT,                      -- 该阶段做了什么（人/agent 可写）
  outcome  TEXT,                      -- 阶段产出/结论
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY(thread_stage_id) REFERENCES thread_stages(id) ON DELETE CASCADE
)

-- 问题/障碍：一等公民，可统计可排序
thread_stage_issues(
  id TEXT PRIMARY KEY,
  thread_stage_id TEXT NOT NULL,
  title TEXT NOT NULL,
  description TEXT,
  status TEXT NOT NULL,        -- open | resolved | dismissed
  severity TEXT NOT NULL,      -- low | medium | high | critical
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY(thread_stage_id) REFERENCES thread_stages(id) ON DELETE CASCADE
)

-- 工作状态快照封套：独立可查实体，原始对话文件丢失也能渲染
thread_work_snapshots(
  child_agent TEXT NOT NULL,
  child_session_id TEXT NOT NULL,     -- 新建 chat 的 session
  thread_id TEXT NOT NULL,
  stage_id TEXT,                      -- stage chat 才有
  snapshot_json TEXT NOT NULL,        -- ThreadWorkSnapshot 序列化
  version INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(child_agent, child_session_id)
)
```

- **`threads.stage_id` 保留**为"当前聚焦指针"（驱动 ProjectPage setThreadStage 交互，src/pages/ProjectPage.tsx:1142-1166），**不再是唯一完成信号**——完成度由 `status` 表达，二者正交。
- **`enabled`**（在 `stages` 模板表）与 `status` 正交：enabled 控制是否参与，status 是进度。

## 状态机（6 态语义）

`not_started`（未开始）→ `in_progress`（进行中）→ `needs_review`（待评审，呼应项目既有 review 工作流）→ `completed`（完成）；旁支 `blocked`（受阻，配 issue 记录细节）、`skipped`（跳过，不计入未完成）。

## 双层快照的关系

- **`session_history_snapshots`（复用，已存在）**：承载对话 turn 连续性（fork 链路那套，src/pages/ChatPage.tsx:235 / src/api.ts:269）。
- **`thread_work_snapshots`（新增）**：承载工作状态封套（progress rollup + issue refs + detail refs）。
- 二者由 `(child_agent, child_session_id)` 关联同一个新 chat：work-snapshot 是"状况主体"，history-snapshot 是"对话证据"。

## `ThreadWorkSnapshot` 内容（snapshot_json）

thread meta（id/project/goal/description/active stage/时间）；ordered stage state（thread_stage_id、project stage id、name/kind/icon、status、summary、outcome、assistants、linked session refs、issue refs+摘要）；rollup（completed/incomplete/blocked 计数、open issue 数、当前 stage 标签）；related context（相关 linked session 的近期对话 excerpt 引用、kanban refs）；**detail refs**（thread/stage/issue/session ids、原始 filePath、`session_history_snapshots` group 索引）——用 source index + 按需 fetch，不在单个响应里塞全部原文。

## API 面（Tauri command + 前端 wrapper，终态 7 个）

`get_thread_work_state(threadId)`、`update_thread_stage_state(threadStageId, patch)`、`create_thread_stage_issue` / `update_thread_stage_issue` / `delete_thread_stage_issue`、`save_thread_work_snapshot` / `get_thread_work_snapshot` / `get_thread_work_snapshot_sources`。写命令返回更新后的对象；GUI 经现有 `threads_updated` 事件刷新，**不存在 agent-only 的独立状态**。

## CLI 面（src-tauri/src/cli.rs 扩 `Command` 枚举，全部 `--json`）

- 读：`sessio thread list/show`、`sessio stage list/show`
- 写状态：`sessio stage set-status --id <threadStageId> --status <6态> [--summary] [--outcome]`、`stage update ...`
- issue：`sessio stage issue add/list/set/delete`
- 复用 `open_store`（src-tauri/src/cli.rs:1232）+ store trait 现成方法（src-tauri/src/store/mod.rs:167-247）。

## Agent 闭环（CLI + skill + 注入）

二进制软链到 `~/.sessio/bin/sessio`（app 启动 `.setup()`（src-tauri/src/lib.rs:2325）里 `current_exe()` 软链）；新增 skill 教 agent 用法；**发起 chat 时把 `threadStageId` + CLI 示例写进上下文**，让 agent 知道"我在哪个 stage、完成后跑哪条命令"，优先 CLI 回写而非只在对话里口述状态。

示例（嵌入 chat 上下文的指令）：

```text
You are working in Sessio thread stage <threadStageId>.
When you begin work:   sessio stage set-status --id <threadStageId> --status in_progress --json
When complete:         sessio stage set-status --id <threadStageId> --status completed --json
If blocked:            sessio stage set-status --id <threadStageId> --status blocked --json
                       sessio stage issue add --stage-id <threadStageId> --title "..." --severity high --json
```

---

# 当前执行骨架（最小可用 · 本轮实现）

## 切线原则

只切一条**端到端最薄、价值最直接**的线：**让 stage 状态可被人和 agent 读写并持久**（直接取代顺序推算）。验证三个架构假设——独立状态表的读写模型、CLI 跨进程写同库、agent 可达性——但不铺 issue UI、不做快照流程。

## 本轮做（骨架范围）

1. **DB（SCHEMA_V6）**：仅建 `thread_stage_states` 表（含 summary/outcome 列，留位）。**不做物化迁移**——采用 lazy 缺省：
   - 读侧 `load_thread_stages`（src-tauri/src/store/sqlite.rs:2847）LEFT JOIN `thread_stage_states`；无行时按"相对 `threads.stage_id` 顺序"现算默认（之前→completed、当前→in_progress、之后→not_started），**不落库**。
   - 写侧 UPSERT 进 `thread_stage_states`。
2. **Rust**：`models.rs` 加 `enum StageStatus`（6 态，仿 `KanbanStatus`，src-tauri/src/models.rs:386）；`StageInfo`（src-tauri/src/models.rs:345）加 `status`/`summary`/`outcome`；`thread_stage_from_row`（src-tauri/src/store/sqlite.rs:1634）+ 两处 SELECT 同步（**风险：列索引对齐**）；新增 command `update_thread_stage_state`（trait/sqlite/cached 三处：src-tauri/src/store/mod.rs:214、src-tauri/src/store/sqlite.rs:4376、src-tauri/src/store/cached.rs:372）。
3. **前端**：src/api.ts:103 `StageInfo` 加字段 + `StageStatus` 类型 + `updateThreadStageState` wrapper；src/pages/ThreadPage.tsx 删 `stageState`/`activeIndex` 推算（:315/:53），改读 `stage.status` 驱动步骤条（6 态各配图标/色，blocked→amber、needs_review→蓝）；stage 卡片头加 status `<select>` 内联编辑。
4. **CLI（最小一条）**：`sessio stage set-status --id <threadStageId> --status <6态> [--summary] [--outcome] --json` + `sessio stage list/show`（验证读写闭环）；扩 `Command` 枚举 + `parse_args` + `print_help`。
5. **可达性**：app 启动软链二进制到 `~/.sessio/bin/sessio`（先 remove 再 create，失败仅告警）。

## 明确推迟（架构已留位，本轮不做）

- `thread_stage_issues` 表 + issue CRUD command + UI（→ 后续 SCHEMA_V7）——骨架阶段"问题"先用 `summary`/`outcome` 文本承载，issue 表上线后再迁移。
- `thread_work_snapshots` 表 + 三个 snapshot command + 发起 chat 的快照采集/回看流程（→ 后续阶段）。本轮**不动**新建 chat 流程；快照沿用现有 `session_history_snapshots` 直到 work-snapshot 上线。
- skill 文档、threadStageId 注入上下文（依赖快照流程，随后续阶段）。
- `summary`/`outcome` 的内联编辑 UI（列已建、CLI 可写，GUI 编辑后补）。

## 验证

- **骨架端到端**：`pnpm tauri dev` → ThreadPage 改某 stage status（含 blocked/needs_review）→ 步骤条即时反映、重启后持久；旧 thread 首次打开按顺序缺省正确显示、无需手动迁移。
- **CLI 闭环**：`cargo build` 后 `~/.sessio/bin/sessio stage set-status --id <id> --status completed --summary "x" --json` 写库成功 → GUI ThreadPage 刷新可见；`sessio stage list --thread-id <id> --json` 可解析。并发：CLI 写时 GUI 开同 thread 不报 locked（确认 `SqliteStore::open` 的 WAL/busy_timeout）。
- **构建**：`pnpm tsc --noEmit` + `cargo build`；`cargo test` 跑迁移断言（版本 5→6）+ thread_stage_states 读写/lazy 缺省单测。
- **架构一致性自检**：骨架建的 `thread_stage_states`、`StageStatus(6 态)`、command 形状，与目标架构定义逐一对齐——后续加 issue/snapshot 表时无需改写已落地部分。
