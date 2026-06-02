# Thread/Stage 工作状态：最终方案（目标架构 + MVP 执行计划）

> **本文是权威实施文档**，融合并取代以下三份的分歧点（旧文档保留备查，不再更新）：
> - `thread-stage-chat-snapshot-plan.md`（B，设计导向，完整建模）
> - `thread-chat-stage-scalable-stonebraker.md`（A，实现导向，精确 file:line）
> - `thread-stage-work-state-fused-plan.md` / `thread-stage-work-state-architecture.md`（两版融合稿）
>
> **立场**：以 B 为**目标架构**、以 A 为**执行地图**。已就三处分歧定稿：
> 1. **issue 表/UI 推迟**到状态闭环之后（MVP 不含）。
> 2. **CLI 对 agent 不开放破坏性删除**（删除留在 GUI/API）。
> 3. MVP 要足够小以安全落地，但**数据模型不能把自己逼进死角**。

---

## Context（为什么做这个）

Sessio 里 thread 是工作流容器、stage 是其中阶段，各自关联若干 agent sessions。三个递进需求：
1. **快照**：从 thread/stage 发起新 chat 时，把"那一刻"整体状况冻结成上下文喂给 agent，回看走冻结快照。
2. **结构化状态**：stage 当前**无任何存储状态**——`threads`/`stages`/`thread_stages` 表无 status 列，ThreadPage 的 done/active/pending 是 `stageState()`（src/pages/ThreadPage.tsx:315）按顺序相对 `thread.stageId` **实时推算**；"问题/障碍"无处可存。需显式建模。
3. **CLI 闭环**：sessio **已有成熟 CLI**（src-tauri/src/cli.rs，src-tauri/src/main.rs:5 `args_os().len()>1` 即走 CLI，与 GUI 同一二进制、开同一个 `~/.sessio/db-data/sessio-index.db`、已有 sessions/memory/config 命令族且全支持 `--json`）。扩展 thread/stage 命令族，让干活的 agent 自己回写状态。

---

# 第一部分：目标架构（方向锚定 · 终态）

> 所有实现不得违背本节。后续每阶段只是往这副骨骼上填肉。Thread/stage 状态是**一等工作流数据**，不是推算出来的 UI 装饰。

## 核心原则

- `threads.stage_id` 保留为**当前聚焦指针**（驱动 ProjectPage setThreadStage 交互，src/pages/ProjectPage.tsx:1142-1166），**不再是唯一完成信号**——完成度由 `status` 表达，二者正交。
- stage 进度**独立显式存储**，不靠顺序推算（推算仅用于迁移/缺省）。
- stage 问题/障碍是**结构化记录**，非仅自由文本。
- 对话 excerpt 留在 `session_history_snapshots`（复用现有 fork 链路）。
- thread/stage 整体状况存 `thread_work_snapshots`（独立可查实体）。
- 细节经 **source index + 按需 API** 获取，不在每个响应里塞全部原文。
- `enabled`（在 `stages` 模板表）与 `status` 正交：enabled 控制是否参与，status 描述进度。

## 状态机（6 态）

`not_started`（未开始）→ `in_progress`（进行中）→ `needs_review`（待评审，呼应项目既有 review 工作流与 `KanbanStatus` 的 agent_review/human_review，src-tauri/src/models.rs:386）→ `completed`（完成）；旁支 `blocked`（受阻，配 issue 记录细节）、`skipped`（跳过，不计入未完成）。

issue status：`open` / `resolved` / `dismissed`；severity：`low` / `medium` / `high` / `critical`。

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

## 双层快照的关系

- **`session_history_snapshots`（复用，已存在）**：承载对话 turn 连续性（src/pages/ChatPage.tsx:235 / src/api.ts:269）。
- **`thread_work_snapshots`（新增）**：承载工作状态封套（progress rollup + issue refs + detail refs）。
- 二者由 `(child_agent, child_session_id)` 关联同一新 chat：work-snapshot 是"状况主体"，history-snapshot 是"对话证据"。

## `ThreadWorkSnapshot` 内容（snapshot_json）

thread meta（id/project/goal/description/active stage/时间）；ordered stage state（thread_stage_id、project stage id、name/kind/icon、status、summary、outcome、assistants、linked session refs、issue refs+摘要）；rollup（completed/incomplete/blocked 计数、open issue 数、当前 stage 标签）；related context（相关 linked session 近期对话 excerpt 引用、kanban refs）；**detail refs**（thread/stage/issue/session ids、原始 filePath、`session_history_snapshots` group 索引）。

## API 面（Tauri command + 前端 wrapper，终态 7 个）

`get_thread_work_state(threadId)`、`update_thread_stage_state(threadStageId, patch)`、`create_thread_stage_issue` / `update_thread_stage_issue` / `delete_thread_stage_issue`、`save_thread_work_snapshot` / `get_thread_work_snapshot` / `get_thread_work_snapshot_sources`。写命令返回更新后对象；GUI 经现有 `threads_updated` 事件刷新，**不存在 agent-only 的独立状态**。

## CLI 面（src-tauri/src/cli.rs 扩 `Command` 枚举，全部 `--json`）

读：`sessio thread list/show`、`sessio stage list/show`
写状态：`sessio stage set-status --id <threadStageId> --status <6态>`、`sessio stage update --id <id> [--status] [--summary] [--outcome]`
issue：`sessio stage issue add/list/set`（**不含 delete**：agent 用 `set --status dismissed` 代替删除，物理删除留给 GUI/API）
复用 `open_store`（src-tauri/src/cli.rs:1232）+ store trait 现成方法（src-tauri/src/store/mod.rs:167-247）。

> **安全边界（定稿）**：CLI **不向 agent 暴露破坏性删除**（thread/stage delete）。删除等破坏性 CRUD 只留在 GUI/API，除非将来引入权限模型。agent 只需 show/list + 状态/issue 写入即可闭环，更安全。

## Agent 闭环（CLI + skill + 注入）

二进制软链到 `~/.sessio/bin/sessio`（app 启动 `.setup()`，src-tauri/src/lib.rs:2325，`current_exe()` 软链）；新增 skill 教用法；**发起 chat 时把 `threadStageId` + CLI 示例写进上下文**，优先 CLI 回写而非只在对话里口述。

```text
You are working in Sessio thread stage <threadStageId>.
When you begin work:   sessio stage set-status --id <threadStageId> --status in_progress --json
When complete:         sessio stage set-status --id <threadStageId> --status completed --json
If blocked:            sessio stage set-status --id <threadStageId> --status blocked --json
                       sessio stage issue add --stage-id <threadStageId> --title "..." --severity high --json
```

---

# 第二部分：MVP 执行计划（分阶段落地）

> 纪律：**每阶段产物都是目标架构的真子集**，绝不骨架一套、终态另一套。

## Phase 1：stage 状态读写闭环（本轮最小可用）★

只切一条**端到端最薄、价值最直接**的线：**让 stage 状态可被 GUI 读写并持久**（Phase 2 再接上 agent CLI），直接取代顺序推算。

1. **DB（SCHEMA_V6）**：当前 repo 最高版本为 SCHEMA_V5（src-tauri/src/store/sqlite.rs:225，`run_migrations` 应用至 version 5，:603-605），故新表落在 **SCHEMA_V6 / version 6**（实施时仍以当时的 `MAX(version)` 为准，避免被并行 PR 抢号）。仅建 `thread_stage_states` 表（含 summary/outcome 列留位）。**不做物化迁移**，采用 lazy 缺省：
   - 读侧 `load_thread_stages`（src-tauri/src/store/sqlite.rs:2847）LEFT JOIN `thread_stage_states`；无行时按"相对 `threads.stage_id` 顺序"现算（之前→completed、当前→in_progress、之后→not_started、无 active→全 not_started），**不落库**。
   - 写侧 UPSERT 进表。
2. **Rust**：`models.rs` 加 `enum StageStatus`（6 态，仿 `KanbanStatus` 的 `as_str`/`from_db_str`，src-tauri/src/models.rs:386）；`StageInfo`（src-tauri/src/models.rs:345）加 `status`/`summary`/`outcome`；`thread_stage_from_row`（src-tauri/src/store/sqlite.rs:1634）+ 两处 SELECT 同步（**风险：列索引对齐**）；新增 command `update_thread_stage_state`（trait/sqlite/cached 三处：src-tauri/src/store/mod.rs:214、sqlite.rs:4376、cached.rs:372）+ `app.emit("threads_updated")`。
3. **前端**：src/api.ts:103 `StageInfo` 加字段 + `StageStatus` 类型 + `updateThreadStageState` wrapper；src/pages/ThreadPage.tsx 删 `stageState`/`activeIndex` 推算（:315/:53），改读 `stage.status` 驱动步骤条（6 态各配图标/色，blocked→amber、needs_review→蓝、skipped→灰虚线）；stage 卡片头加 status `<select>` 内联编辑。

## Phase 2：CLI 状态更新（agent 闭环）★

扩现有手写 parser 加 `thread`/`stage` 命令族（src-tauri/src/cli.rs）。
- 复用同一 SQLite store；每个读写命令支持 `--json`。
- **非破坏性命令起步**：thread/stage show/list、stage set-status/update（status/summary/outcome）。
- 写命令返回更新后的 stage 对象。
- 二进制软链 `~/.sessio/bin/sessio`（先 remove 再 create，失败仅告警不阻塞启动）。
- 新增 skill / 项目级 agent 指令教 agent 读写自己的 stage。

> **★ Phase 1 + Phase 2 = 第一个有用里程碑**：持久化的 stage 状态 + agent 可经 CLI 更新。

## Phase 3：新 chat 的 work-state 快照

状态与 CLI 落地后再做 thread/stage chat 上下文。
- 新 thread chat 关联 thread + active stage；新 stage chat 关联 thread + 选定 stage。
- agent 上下文含 `threadId`/`threadStageId`/status/summary/outcome/**已知问题文本（取自 summary/outcome；Phase 4 后替换为结构化 open issues）**/linked session refs + CLI 示例。
- 宽 work-state 封套存 `thread_work_snapshots`；对话 excerpt 存现有 `session_history_snapshots`。
- 原始 chat 文件后续不可用时，快照展示仍稳定。

## Phase 4：issue 建模 + detail sources + 打磨

> **issue 表/UI 定稿推迟至此**（不进 MVP；Phase 1-3 阶段"问题"先用 `summary`/`outcome` 文本承载）。

- SCHEMA_V7 建 `thread_stage_issues` 表 + issue command（create/update + **delete 仅 GUI/API**；agent CLI 只有 add/list/set，删除靠 `status=dismissed`）+ ThreadPage issue UI。
- work snapshot 的 source/details 视图；`get_thread_work_snapshot_sources` 返回 thread/stages/issues/linked sessions/file paths/excerpt group 索引的标签与 refs。
- 用现有 `getSessionHistory` 下钻原始 session。
- rollup 完善：completed/blocked/open issue 计数、当前 stage 标签。

---

## 取舍与决策（定稿）

- **不只用 `note`**：快但多障碍、severity、解决态、过滤、可审计性都脆弱 → 终态用独立 issue 表（Phase 4）。
- **不只用 `session_history_snapshots`**：适合对话 excerpt，但对持久工作流状态语义错误 → 用 `thread_work_snapshots`。
- **v1 不让 agent 做破坏性 CRUD**：status/summary/outcome 写入足够支撑第一版闭环；issue 写入在 Phase 4 加入后仍保持非破坏性边界（add/set，不含 delete）。
- **保留 `threads.stage_id`**：仍是有用的聚焦指针，保留现有 stage 激活行为。
- **`enabled` 与 status 分离**。
- **显式 status 优先于推算**：推算仅用于迁移/缺省。

## 验收标准

- 用户能在 GUI 把 stage 设为 `blocked`/`completed`（Phase 1）。
- agent 能跑 `sessio stage set-status --id <id> --status completed --json`，GUI 刷新后反映（Phase 2）。
- 新 thread/stage chat 含 work-state 上下文 + CLI 指令（Phase 3）。
- agent 能经 CLI 加 issue 且显示在该 stage 下（Phase 4）。
- 保存的快照保留 stage 状态/issues/summary/outcome/linked sessions/source refs；原始 session 文件消失后概览仍渲染（Phase 3-4）。

## 验证

- **Phase 1 端到端**：`pnpm tauri dev` → ThreadPage 改某 stage status（含 blocked/needs_review）→ 步骤条即时反映、重启后持久；旧 thread 首次打开按顺序缺省正确、无需手动迁移。
- **Phase 2 CLI 闭环**：`cargo build` 后 `~/.sessio/bin/sessio stage set-status --id <id> --status completed --summary "x" --json` 写库成功 → GUI 刷新可见；`sessio stage list --thread-id <id> --json` 可解析。并发：CLI 写时 GUI 开同 thread 不报 locked（确认 `SqliteStore::open` 的 WAL/busy_timeout）。
- **构建**：`pnpm tsc --noEmit` + `cargo build`；`cargo test` 跑迁移断言（版本 5→6）+ `thread_stage_states` 读写/lazy 缺省单测。
- **架构一致性自检**：Phase 1 建的 `thread_stage_states`、`StageStatus(6 态)`、command 形状，与第一部分目标架构逐一对齐——后续加 issue/snapshot 表时无需改写已落地部分。
