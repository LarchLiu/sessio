# Phase 4：issue 建模 + ThreadChatPage + 快照打磨

> 历史说明：本文档描述的是旧的 stage snapshot / work overview `ThreadChatPage` 路径。New Chat 的四种 thread mode 入口不再以该页为最终落点；最新规则见 `docs/new-chat-thread-mode-entry-plan.md`，四种 thread mode 都跳转 `ThreadMultiSessionChatPage`，普通 New Chat 才进入 `ChatPage`。

## Context（为什么做）

Sessio 的 thread/stage 工作状态已走完 Phase 1-3：6 态 `StageStatus` 可经 GUI/CLI 读写持久（取代顺序推算），新 chat 能注入并保存 `thread_work_snapshots`。但还有三处缺口：

1. **「问题/障碍」无结构化载体**——只能塞进 stage 的 `summary`/`outcome` 自由文本，无法按 severity/status 统计、过滤、审计。

2. **工作快照只存不显示**——`getThreadWorkSnapshot` 至今没有任何前端消费者，「从 thread 发起的 chat 是在什么工作状态下发起的」无处可看。

3. **从 thread 发起 chat 体验错位**——当前复用 NewChatPage（经 `snapshotContext`），底部却是与 thread 无关的 **kanban item 选择器**。

Phase 4 补齐：issue 成为一等结构化记录（表 + CRUD + CLI + GUI 完整管理）；新建 **ThreadChatPage** 取代错位复用——底部改为 **选 thread**、页内展示该 thread 工作状态概览并可下钻原始对话；快照注入与 rollup 改用结构化 open issues。

**权威文档**：`docs/thread-stage-work-state-final.md`（Phase 4 在 154-161 行）。严格遵循其目标架构：issue `status ∈ {open, resolved, dismissed}`、`severity ∈ {low, medium, high, critical}`；**CLI 对 agent 不开放 delete**（删除靠 `set --status dismissed`，物理删除仅 GUI/API）。当前 schema 最高 = **V7**，issue 表落 **SCHEMA_V8**。

**用户已拍板的 3 个决策**：

1. 新建 ThreadChatPage，**沿用 ThreadPage「新建 chat」入口**，thread 选择器默认当前 thread、同项目可切换；NewChatPage 保持不变。

2. ThreadChatPage 选 thread 后展示**工作状态概览**（stages status / open issues / linked sessions）+ 点 session **页内下钻原始对话**。

3. issue UI **完整管理**（增 / 改 status·severity / 物理删）。

***

## 实现（分阶段，每阶段为目标架构真子集）

### P4-A 后端数据层（issue 表 + 模型 + store）

文件：`src-tauri/src/{models.rs, store/mod.rs, store/sqlite.rs, store/cached.rs}`

* **models.rs**：加 `IssueStatus`、`IssueSeverity` enum（仿 `KanbanStatus`:425-459 的 `as_str`/`from_db_str`）；`StageIssueInfo` struct（camelCase serde，仿 `KanbanItem`）；`StageInfo`(379-406) 加 `#[serde(default)] pub issues: Vec<StageIssueInfo>`。

* **sqlite.rs**：

  * `const SCHEMA_V8`：建 `thread_stage_issues`（文档 DDL，FK→thread_stages(id) ON DELETE CASCADE）+ `CREATE INDEX IF NOT EXISTS idx_thread_stage_issues_stage`。

  * `run_migrations`(602-671)：在 663 之后、`seed_*`(664) 之前加 `if current < 8 { conn.execute_batch(SCHEMA_V8)?; INSERT OR IGNORE … VALUES (8); }`（**无&#x20;**`let _ =`——新表不在 V5 bootstrap）。

  * `issue_from_row`（仿 `kanban_item_from_row`:1583）+ `load_stage_issues(conn, thread_stage_id)`（`ORDER BY created_at ASC`）。

  * `load_thread_stages` for 循环(2956-2971) 加 `stage.issues = load_stage_issues(conn, &stage.id)?;`；`load_thread_stage_by_id`**&#x20;同步填充 issues**（写命令返回值，漏填会导致 GUI CRUD 后对象缺 issues）。

  * 4 个 trait 方法实现（仿 kanban CRUD:4849-4955）：`list/create/update/delete_thread_stage_issue`。create：title trim 非空、id 仿 `stable_kanban_id`、status 默认 open、`now_ms()` 双写；update：`Option`/`Option<Option<&str>>` 合并 + 刷新 `updated_at`；delete：`changed == 0` 则 bail。

  * **顺手在&#x20;**`open()`**(36-40) 加&#x20;**`PRAGMA busy_timeout = 5000;`——issue 写入会加剧 GUI/CLI 写写竞争，当前无 busy_timeout 会偶发 `SQLITE_BUSY`。

* **store/mod.rs**：`SessionStore` trait 加 4 个签名（仿 kanban:271-285）。

* **store/cached.rs**：加 4 个纯转发（仿 452-499）。

### P4-B Tauri command + CLI

文件：`src-tauri/src/{lib.rs, cli.rs}`

* **lib.rs**：4 个 `#[tauri::command]`（仿 `update_thread_stage_state`:674-697——`status`/`severity` 用 `Option<String>` 经 `from_db_str` 转 enum + 「invalid …」错误，`description` 空串清空）；create/update/delete 三个写命令 `app.emit("threads_updated", ())`；注册进 `invoke_handler`(2546-2622)。

* **cli.rs**：`StageCommand`(46-66) 加 `Issue(IssueCommand)`；新 `enum IssueCommand { Add, List, Set }`（**无 Delete**）；`parse_stage`(1420-1530) 加 `"issue"` 分支 → `parse_stage_issue`（index-loop，仿 set-status:1482-1527）；`run_stage`(724-797) 加 `Issue` 分支 → `run_stage_issue`（`--json` 用 `to_string_pretty`，Set 校验 status/severity）；`print_help`(1779 后) 加 3 行 `sessio stage issue add/list/set …`。

### P4-C 前端 issue 类型 + ThreadPage UI

文件：`src/{api.ts, pages/ThreadPage.tsx, utils/stageDisplay.ts, i18n.tsx}`

* **api.ts**：`StageIssueInfo` + `IssueStatus`/`IssueSeverity` 类型（103-109 旁）；`StageInfo`(111-133) 加 `issues`；4 个 wrapper（仿 `updateThreadStageState`:912-926）。

* **utils/stageDisplay.ts**：把 `stageStatusVisual`/`STAGE_STATUS_ORDER`/`stageLabel`（现于 ThreadPage:334-396）抽入此已存在文件，ThreadPage 与 ThreadChatPage 共用（避免第二份 6 态视觉表）。

* **ThreadPage.tsx**：`ThreadStageStep` 卡片在 assistants lane(232) 之后加 issue 区——列 issues（severity 色标 + status 标签）、新增、改 status(3 态)/severity、删除；经 wrapper + `reload`(30-34) 刷新。

* **i18n.tsx**：issue 文案（en 153 区 / zh 403 区）。

### P4-D composer 复用抽取（为 ThreadChatPage 铺路，先保证 NewChatPage 不回归）

文件：`src/hooks/useChatComposer.ts`(新)、`src/components/ChatComposer.tsx`(新)、`src/pages/NewChatPage.tsx`

* **useChatComposer**：封装零上下文耦合的 composer 内核——agent/model/effort/permission state + 3 个同步 effect + attachments(`useComposerAttachments`) + handlers + `runStartSession(prompt, { extraContext })`（封装 `startAgentSession → rememberRuntimeAgentSelection → dispatchSessionStartedFallback → sendAgentInput`，`extraContext` 即 NewChatPage:426-428 的快照拼接骨架）。

* **ChatComposer**：composer 视觉壳（错误条 + attachment preview + textarea/Enter 发送 + 工具栏行，NewChatPage:452-534）+ `bottomRow?: ReactNode` 插槽（对应底部选择器行 535-562）。`ScrambledProjectName`/`NewChatMenuButton`/`resizeTextareaToContent` 一并移入。

* **NewChatPage** 改用二者，**行为完全不变**（独立可回滚验证节点）；移除 `snapshotContext` prop 与 workSnapshot 分支（该职责迁往 ThreadChatPage，净简化）。

### P4-E ThreadChatPage + AppMain 接入

文件：`src/pages/ThreadChatPage.tsx`(新)、`src/components/SessionHistoryReadonly.tsx`(新)、`src/components/AppMain.tsx`

* **ThreadChatPage**：基于 useChatComposer + ChatComposer。底部 `bottomRow` = project 选择器 + **thread 选择器**（默认 `snapshotContext.thread`，`listThreads(project.id)` 拉同项目全部 thread，切换在已加载列表内 `find`、无需二次请求）。**ThreadWorkOverview 面板**：rollup 行 + 每 stage（`stageDisplay` 图标/色 + status + open issues 数 + linked sessions）。点 session → `getSessionHistory`(api.ts:1165) 页内下钻。`handleSend` 用 `buildThreadWorkSnapshot(选中 thread, focusedStage)` + `runStartSession(prompt, { extraContext: renderThreadWorkContext(snapshot) })`，`onPendingSession` 带 workSnapshot（落地仍走 usePendingNewChats:113-119）。

* **SessionHistoryReadonly**：遍历 `turns[].blocks`(`SessionHistoryRenderBlock`，api.ts:305-312)只读渲染——user/assistant/thought 取文本走**复用 ChatPage 现有 markdown 渲染封装**（`react-markdown` 栈已在依赖；实现时定位该封装组件复用样式），tool/permission/error/sessionUpdate 折叠为图标+类型标签。`!session.filePath`（刚发起的 partial）置灰提示无历史。不接 live/打字机。

* **AppMain.tsx**：`!selected` 分支(149-163)按 `newChatSnapshot` 分流——非空渲染 `ThreadChatPage`，否则 `NewChatPage`。`openNewChatForStage`(93-99) 入口路径不变（满足决策 1）。可选透传 `onSelectSession` 让下钻「展开为完整 ChatPage」。

### P4-F 快照结构化 issues + rollup 完善

文件：`src/{api.ts, threadSnapshot.ts}`

* **api.ts**：`ThreadWorkSnapshotStage` 加 `issues`；`ThreadWorkSnapshot.rollup` 加 `openIssues: number` + `currentStage: string | null`。

* **threadSnapshot.ts**：`snapshotStage` 带 issues；`buildThreadWorkSnapshot`(31-58) rollup 算 `openIssues`（全 stage open issue 计数）+ `currentStage`；`renderThreadWorkContext`(82-119) 每个 stage 在 summary/outcome 后追加结构化 open issues（`issue [high] <title>`，取代 Phase 3 临时用 summary/outcome 承载问题），Progress 行追加 open issues 数。

***

## 复用的现有件

* enum 模式：`KanbanStatus`（models.rs:425-459）

* CRUD / row 映射 / 填充：kanban CRUD（sqlite.rs:4849-4955）、`kanban_item_from_row`(1583)、`stable_kanban_id`、`load_thread_stages` 的 assistants/sessions 填充(2956-2971)

* command / CLI 模式：`update_thread_stage_state`（lib.rs:674-697）、stage set-status parse/run（cli.rs:1482-1527 / 771-795）

* 快照：`buildThreadWorkSnapshot`/`renderThreadWorkContext`（threadSnapshot.ts:31-119）、`usePendingNewChats` 保存链路(113-119)

* markdown：`react-markdown` 栈（package.json 已含）+ ChatPage 现有渲染封装

## 风险

* `load_thread_stage_by_id`**&#x20;必须同步填 issues**（写命令返回值），否则 GUI CRUD 后对象缺 issues。

* **V8 迁移**：`execute_batch(…)?`（无 `let _`）+ `IF NOT EXISTS` + `INSERT OR IGNORE`，幂等。

* **busy_timeout**：加 `PRAGMA busy_timeout = 5000`，缓解 GUI/CLI 写写竞争。

* **composer 抽取回归（最高关注）**：P4-D 先让 NewChatPage 切内核并单独验证（可回滚）再做 P4-E。回归重点：agent/model 切换持久化、permission 校正 effect、Enter 发送、attachments、kanban 链接 + todo→in_progress 跃迁。

* **下钻保真**：SessionHistoryReadonly 为概览级只读；partial/无 filePath 置灰；完整体验走 onSelectSession 跳 ChatPage。

## 端到端验证

1. `cargo build` + `cargo test`：新增 V7→V8 迁移断言、issue CRUD + FK CASCADE（删 thread_stage 后 issues 随之消失）、`load_thread_stages` 填充 issues 单测。

2. `pnpm typecheck`（tsc --noEmit）。

3. **CLI 闭环**：`sessio stage issue add --stage-id <id> --title "x" --severity high --json` → `issue list --stage-id <id> --json` → `issue set --id <issueId> --status dismissed --json`；确认 `issue delete` 报 unknown（无破坏性命令）；并发：CLI add 时 GUI 开同 thread 不报 `SQLITE_BUSY`。

4. `pnpm tauri dev`**&#x20;手测**：ThreadPage issue CRUD（3 态/4 级/物理删，即时刷新）；从 stage「新建 chat」进 ThreadChatPage（默认当前 thread，切同项目其它 thread 概览重载，显示 status + open issues + sessions）；点 session 页内只读下钻（partial 置灰）；选 agent/加附件/发送 → 注入文本含结构化 open issues + rollup（openIssues/currentStage）+ CLI 指令；NewChatPage 项目级新 chat 回归（kanban 链接 + todo→in_progress）。

5. **重启**：issue 持久；既有快照仍渲染（Phase 3 不回归）。
