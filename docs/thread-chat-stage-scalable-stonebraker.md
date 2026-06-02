# Thread / Stage 状态字段 + CLI + Chat 快照

## Context（为什么做这个）

Sessio 里 thread 是工作流容器、stage 是其中阶段，各自关联若干 agent sessions。用户要让 thread/stage 拥有**真实可维护的状态**，并能把"发起新 chat 那一刻"的整体状况冻结成快照喂给新 agent，还要让**正在干活的 agent 通过 sessio CLI 自己更新 stage 状态**，形成工作流闭环。

三个递进需求（用户逐步明确）：
1. **快照**：从 thread/stage 发起 thread chat / stage chat，把状况冻结成上下文，回看走冻结快照。
2. **结构化状态**：stage 当前**没有任何存储的状态字段**（`threads`/`stages`/`thread_stages` 表无 status/note 列；ThreadPage 的 done/active/pending 是 [stageState()](src/pages/ThreadPage.tsx#L315) 按顺序相对 `thread.stageId` **实时推算**的）。需给 stage 加 `status` 枚举 + `note` 文本，**取代顺序推算**。
3. **CLI 写状态**：sessio **已有成熟 CLI**（[cli.rs](src-tauri/src/cli.rs)，[main.rs:5](src-tauri/src/main.rs#L5) `args_os().len()>1` 即走 CLI，与 GUI 同一二进制，`open_store` 开同一个 `~/.sessio/db-data/sessio-index.db`，已有 sessions/memory/config 命令族且全支持 `--json`）。扩展出 thread/stage 命令族让 agent 调用。

已确认决策：CLI 做**完整 thread/stage CRUD**；agent 可达性靠 **skill + 固定路径**（`~/.sessio/bin/sessio`）；**三块全做**，顺序为 P1 字段+CLI → P2 skill 可达性 → P3 快照。

---

## Phase 1：stage status + note 字段（全链路）

### 1.1 DB schema（[sqlite.rs](src-tauri/src/store/sqlite.rs)）
- 已确认有**版本化迁移机制**：`SCHEMA_Vn` 常量 + `schema_migrations(version)` 表，`run_migrations`（sqlite.rs:563 附近）幂等应用，fresh-install 重复 ALTER 用 `let _ = execute_batch(...)` 吞掉。
- 新增 **SCHEMA_V6**：`ALTER TABLE thread_stages ADD COLUMN status TEXT NOT NULL DEFAULT 'pending'; ADD COLUMN note TEXT;`，同时把这两列补进 V1 bootstrap 的 `thread_stages` CREATE TABLE（[sqlite.rs:459](src-tauri/src/store/sqlite.rs#L459)）。
- **回填**（同一迁移块内，幂等 `WHERE status='pending'`）：按旧的"相对 `threads.stage_id` 顺序"——之前的 stage→`done`、当前→`in_progress`、之后→`pending`，避免已有 thread 视觉上全被重置。

### 1.2 Rust（[models.rs](src-tauri/src/models.rs) / [sqlite.rs](src-tauri/src/store/sqlite.rs) / [lib.rs](src-tauri/src/lib.rs)）
- `models.rs`：新增 `enum StageStatus { Pending, InProgress, Blocked, Done }`（仿 [KanbanStatus:386](src-tauri/src/models.rs#L386) 的 `as_str`/`from_db_str` + `#[serde(rename_all="snake_case")]`）；`StageInfo`（[models.rs:345](src-tauri/src/models.rs#L345)）加 `status: StageStatus` + `note: Option<String>`。
- 读取链路：`load_thread_stages` 的 JOIN SELECT（[sqlite.rs:2849](src-tauri/src/store/sqlite.rs#L2849)）和 `load_thread_stage_by_id` 的 SELECT 都补 `ts.status, ts.note`（成 col 15/16）；`thread_stage_from_row`（[sqlite.rs:1634](src-tauri/src/store/sqlite.rs#L1634)）补两个 `row.get`。**风险：列索引对齐**，两处 SELECT 必须与 from_row 一致。
- 写入链路：`update_thread_stage` 三处签名同步——trait（[mod.rs:214](src-tauri/src/store/mod.rs#L214)）、sqlite 实现（[sqlite.rs:4376](src-tauri/src/store/sqlite.rs#L4376)）、cached 包装（[cached.rs:372](src-tauri/src/store/cached.rs#L372)）——加 `status: Option<StageStatus>, note: Option<Option<String>>`（双 Option 区分"不改"与"清空为 NULL"）；在已有 UPDATE（[sqlite.rs:4440](src-tauri/src/store/sqlite.rs#L4440)）里一并写。**注意 `enabled` 写在 `stages` 模板表、与此正交，勿动。**
- command `update_thread_stage`（[lib.rs:657](src-tauri/src/lib.rs#L657)）加 `status: Option<String>` + `note: Option<Option<String>>`，`from_db_str` 解析后转发；已有 `app.emit("threads_updated")` 自动刷新前端。

### 1.3 前端 TS + UI
- [api.ts:103](src/api.ts#L103) `StageInfo` 加 `status` + `note`；新增 `export type StageStatus`（仿 [api.ts:154](src/api.ts#L154) KanbanStatus）；`updateThreadStage`（[api.ts:885](src/api.ts#L885)）patch 加 `status?`/`note?` 并透传。
- [ThreadPage.tsx](src/pages/ThreadPage.tsx)：删除 `stageState`（:315）和 `activeIndex` 推算（:53），改由 `stage.status` 驱动视觉态（done→done、in_progress→active、pending→pending、**blocked→新 amber 变体**）；`ThreadStageStep`（:123）图标（:141）加 blocked 分支、连接线着色（:142-165）改判 `status==='done'`、"active"标签（:175）改判 `status==='in_progress'`。
- **内联编辑**：stage 卡片头部加小 `<select>`（4 状态）→ `updateThreadStage(stage.id,{status})`；note 内联可编辑 textarea → `updateThreadStage(stage.id,{note})`。靠 `threads_updated` 重取。

### 决策答复
- **枚举值** `pending/in_progress/blocked/done`：blocked 表"受阻有问题"，note 承载细节。
- **`thread.stageId` 保留**（与 status 各司其职：stageId=用户当前聚焦指针，驱动 ProjectPage [setThreadStage 点击交互:1142-1166](src/pages/ProjectPage.tsx#L1142)；status=客观完成度）。合并会破坏 chip 的 lock 交互，blast radius 最小化。
- **`enabled` 与 `status` 正交**：enabled 控制 stage 是否参与，status 是进度。

---

## Phase 2：sessio CLI thread/stage 命令族 + agent 可达性

### 2.1 CLI 命令（[cli.rs](src-tauri/src/cli.rs)，复用现有手写 parser 风格）
- `Command` 枚举（[cli.rs:23](src-tauri/src/cli.rs#L23)）加 `Thread(ThreadCommand)` / `Stage(StageCommand)`；`parse_args`（[cli.rs:731](src-tauri/src/cli.rs#L731)）加 `"thread"`/`"stage"` 分支；`run`（[cli.rs:175](src-tauri/src/cli.rs#L175)）加对应 `run_thread`/`run_stage`。
- 全部 `open_store(db_path)`（[cli.rs:1232](src-tauri/src/cli.rs#L1232)）复用同一库，调 store trait 现成方法（[mod.rs:167-247](src-tauri/src/store/mod.rs#L167)：`list_threads`/`create_thread`/`update_thread`/`delete_thread`/`add_thread_stage`/`update_thread_stage`/`delete_thread_stage`/`set_thread_stage`/`link_stage_session`/`unlink_stage_session` 等）。每命令支持 `--json`（与现有约定一致："stable machine-readable output for skills and agents"）。
- **完整 CRUD 子命令**（举例，全部 `--db-path` 可选）：
  - `sessio thread list --project <path>` / `thread show --id <threadId>` / `thread create --project <path> --goal <text>` / `thread set-stage --thread-id <id> --stage-id <threadStageId>`
  - `sessio stage list --thread-id <id>` / `stage show --id <threadStageId>`
  - **`sessio stage set-status --id <threadStageId> --status <pending|in_progress|blocked|done> [--note <text>]`** ← agent 标状态的核心
  - `sessio stage set-note --id <threadStageId> --note <text>` / `stage add --thread-id <id> --stage-id <projectStageId>` / `stage remove --id <threadStageId>` / `stage link-session --id <threadStageId> --agent <a> --session-id <id>`
- `print_help`（[cli.rs:1371](src-tauri/src/cli.rs#L1371)）补这些用法。

### 2.2 二进制可达性（固定路径 + skill）
- **软链到固定路径**：app 启动 `.setup()`（[lib.rs:2325](src-tauri/src/lib.rs#L2325)）里把当前可执行文件（`std::env::current_exe()`）软链/复制到 `~/.sessio/bin/sessio`（`~/.sessio` 目录创建模式已遍布，如 lib.rs:250/2334）。这样无论 app 装在哪，agent 都能用稳定绝对路径调用。
- **skill 文档**：新增一个 skill（`.claude/skills/` 或项目约定位置）告诉 agent：用 `~/.sessio/bin/sessio stage set-status --id <threadStageId> --status done --note "..."` 更新所负责 stage 的状态；并说明 `--json` 读当前状况、threadStageId 从哪获取（见下）。
- **threadStageId 注入**：agent 要知道自己在哪个 stage 干活。建议在 stage chat 发起时（Phase 3），把 `threadStageId` 写进喂给 agent 的上下文 markdown（"你正在 stage <id> 工作，完成后可执行 sessio stage set-status --id <id> ..."）。

### 风险
- 软链权限/已存在旧链接需处理（先 remove 再 create，失败仅告警不阻塞启动）。
- agent 写库与 GUI 并发：SqliteStore 已是进程内锁，跨进程靠 SQLite 文件锁；CLI 是短命进程，冲突窗口小，但需确认 `open` 用 WAL/busy_timeout（查 SqliteStore::open）。

---

## Phase 3：thread chat / stage chat 快照（复用 fork 链路）

> 此阶段在 P1 落地后，快照直接读 `StageInfo.status`/`note` 结构化字段，**不再从对话规则抽取**。

### 复用机制
- 载体 `SessionHistorySnapshotGroup`（[api.ts:269](src/api.ts#L269)）靠 `ancestorIndex` 支持多 group：状况概要占 index 0、各关联 session 完整对话占 index 1..N。
- 落盘（零改动）：[usePendingNewChats.ts:80-110](src/hooks/usePendingNewChats.ts#L80) 自动 `saveSessionHistorySnapshots`。
- 喂上下文：`crossContextAttachment`（[ChatPage.tsx:3653](src/pages/ChatPage.tsx#L3653)，需抽到 [cross.ts](src/cross.ts) 导出）→ `writeCrossPrompt` 落 .md 附件（16KB 上限自动截断）。
- 冻结回看（零改动）：[ChatPage.tsx:725-754](src/pages/ChatPage.tsx#L725) + `snapshotGroupsToAncestorHistoryGroups`（[ChatPage.tsx:235](src/pages/ChatPage.tsx#L235)）。

### 实现
- 新文件 [src/threadSnapshot.ts](src/threadSnapshot.ts)：`buildStatusReport(thread, stage?)` 读 `status`/`note`/各 stage 名单 + 每个 session 的 `[agent:sessionId] 标题` 标识，拼成 index 0 markdown group（含一句"你正在 stage <threadStageId> 工作，可用 sessio stage set-status 更新状态"）；`collectThreadSnapshot(thread, stage?, agent)` 并发 `getSessionHistory` 拉各 session turns 组 index 1..N。
- [NewChatPage.tsx](src/pages/NewChatPage.tsx)：加 `snapshotContext?: {thread, stage?}` prop；`handleSend`（[:363](src/pages/NewChatPage.tsx#L363)）发送前 `collectThreadSnapshot`，groups 进 `onPendingSession.historySnapshots`（[:401](src/pages/NewChatPage.tsx#L401)），每 group 经 `crossContextAttachment` 产独立 .md 附件并入 `sendAgentInput`（[:413](src/pages/NewChatPage.tsx#L413)）。
- thread chat 入口：[AppMain.tsx:105](src/components/AppMain.tsx#L105) `onNewThreadChat(thread)` 存 `setNewChatSnapshot({thread})`，传 `snapshotContext` 给 NewChatPage。
- stage chat 新入口：[ThreadPage.tsx](src/pages/ThreadPage.tsx) 每个 stage 行加按钮 + `onNewStageChat(thread, stage)`，透传 [AppMain.tsx:121](src/components/AppMain.tsx#L121) → `setNewChatSnapshot({thread, stage})`；i18n 补 `stage.new_chat`。

---

## 验证

- **P1**：`pnpm tauri dev`；ThreadPage 改 stage status 下拉/note → 步骤条图标与高亮按 status 变化（blocked 显 amber）；重启 app 状态持久（迁移+回填生效）；旧库升级不报错。`cargo test` 跑迁移断言（版本号 5→6）。
- **P2**：`cargo build` 后 `~/.sessio/bin/sessio stage set-status --id <id> --status done --note "x" --json` 写库成功，GUI ThreadPage 刷新可见；`sessio stage list --thread-id <id> --json` 输出可解析；`sessio thread/stage --help` 文档完整。并发：CLI 写时 GUI 打开同 thread 不报 locked。
- **P3**：thread/stage chat 发起 → 新 chat 祖先区出现状况概要（status/note 正确、session 标识齐全）+ 各 session 完整对话；附件含 .md；发起后改源 session，重开 chat 回看快照内容不变（冻结）；边界：无 session 的 thread、5+ session（截断注明）、不可用 session 降级不报错。
- **构建**：`pnpm tsc --noEmit` + `cargo build` 通过。
