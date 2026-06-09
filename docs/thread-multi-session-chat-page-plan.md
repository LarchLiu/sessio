# Thread Multi-Session Chat Page 分阶段实施方案

## 摘要

新增独立页面 `ThreadMultiSessionChatPage`，用于在同一个 thread 中同时展示多个 sessions 的 history message 和 live message。页面默认采用分容器 lanes：每个 session 一个独立 panel、独立滚动、独立 live 数据源，避免多个 agent/session 同时流入时显示串流。

当前代码里 `ThreadKind = process | teamwork | brainstorm | debate`、plan rounds/tasks、`getThreadReplay`、Astra run、brainstorm/debate dedicated backend 都已存在；新页面 v1 直接基于这些事实源实现。现有 `ThreadChatPage` 保留，新的页面作为独立入口加入，不替换旧页。

## 核心设计

- 新建 `src/pages/ThreadMultiSessionChatPage.tsx`。
- 数据源：
  - `getThreadWorkState(threadId)` 获取 thread kind、stages、assistants、当前状态。
  - `getThreadReplay(threadId)` 获取 thread/stage/plan_task/astra_internal sessions 聚合结果。
  - `listPlanRounds(threadId)`、`listAstraRuns(threadId)` 获取 plan/task 和 Astra 运行状态。
  - `liveState.sessions` + `runtimeSessionAliases` 绑定正在 live 的 session。
  - `pendingNewChats` 补齐尚未拿到真实 agent session id 的本页新会话。
- 展示模型：

```ts
interface ThreadSessionLane {
  laneId: string;
  agent: Agent;
  sessionId: string;
  sessioRuntimeSessionId: string | null;
  session: SessionInfo | null;
  sources: ThreadReplaySessionSourceInfo[];
  groupKey: string;
  groupLabel: string;
  status: "history" | "live" | "pending" | "missing" | "failed";
}
```

- 每个 lane 独立渲染：
  - header：agent、session title、source badges、message count、live status、open full chat。
  - body：先加载 history turns，再 merge 对应 live turns。
  - scroll controller 每个 lane 独立，不能共享 cache key。
- 分组规则沿用现有 replay 语义：
  - `process`：优先按 stage 分组。
  - `teamwork` / `brainstorm`：优先按 plan round 分组。
  - `debate`：优先按 round + lane/task 分组。
  - fallback：按 thread direct / astra internal / agent 分组。

## 关键边界

- 新页面是 thread-level 视图，不改变单 session `ChatPage` 的路由和行为。
- `getThreadReplay` 后端已按 `(agent, sessionId)` 聚合 source；前端不要二次拆开同一个 session，只补充 groupKey 和显示信息。
- live event 必须只通过 `sessioRuntimeSessionId` 路由到对应 lane，不做全局消息拼接。
- `SessionHistoryReadonly` 只能作为静态历史临时空壳；支持 streaming/tool/permission 时必须复用或抽出 `ChatPage` 的 ACP transcript renderer。
- `brainstorm` / `debate` 当前已有 dedicated backend，可接入 Astra run；UI 文案要表达其 shared-board / isolated-lane 语义，而不是置灰为未支持。
- `process` 不走 teamwork 的 LLM automatic scheduling；process Astra run 由 deterministic backend 按 stage 顺序执行，页面仍保留手动 stage task 入口。

## 实施阶段

### Phase 1: 导航状态与页面入口

目标：新增独立页面状态，不影响现有 `ThreadPage` / `ThreadChatPage` / `ChatPage`。

执行内容：

- 扩展 `DetailMode` 或引入更明确的 detail route，例如：

```ts
type DetailMode = "chat" | "project" | "threadMultiSessionChat";
```

- 在 `App.tsx` selection/header 语义中区分：
  - thread overview：当前 `ThreadPage`。
  - thread multi-session chat：新的 `ThreadMultiSessionChatPage`。
  - single session chat：当前 `ChatPage`。
- 在 `AppMain` 增加 `threadMultiSessionChat` 分支，不能只依赖 `activeProject && selectedThreadId` 的现有 `ThreadPage` 分支。
- 在 `ThreadPage` 增加入口按钮，打开当前 thread 的 multi-session chat page。
- `ProjectWorkbenchPage` 的 thread 卡片可后续增加入口；v1 至少保证 `ThreadPage` 可进入。
- 页面首屏展示 thread title、kind、status summary、actions 区、session lanes 区。
- 加载失败、thread missing、empty replay 都要有明确空态。

验收：

- 从 `ThreadPage` 可打开新页面，也能返回 thread overview。
- 选择普通 session、打开项目 workbench、打开原 `ThreadChatPage` 都不被新 route 误切。
- header/sidebar 中 thread 选择态仍正确。
- `process`、`teamwork`、`brainstorm`、`debate` thread 都能进入页面并显示正确 kind。

### Phase 2: Transcript Renderer 最小抽取

目标：在实现多 lane live 前，先避免复制 `ChatPage` 的 ACP 渲染大块逻辑。

执行内容：

- 从 `ChatPage` 抽出最小可复用组件和 helper：
  - `AcpTranscriptPanel`
  - `AcpRenderItems`
  - `LiveSessionStatusBadge`
  - tool/permission 渲染组件
  - history/live view model merge helper
- `ChatPage` 继续使用抽出的 renderer，并保持原有单 session 行为不变。
- `ThreadMultiSessionChatPage` 后续只负责 thread-level layout、lane 数据绑定、actions，不复制 transcript 细节。
- 多 lane 页面使用独立 scroll controller，或让 `AcpTranscriptPanel` 接收稳定 `scrollKey`。

验收：

- `ChatPage` 原有单 session 发送、resume、permission、tool 展示不变。
- 抽出组件能同时接收 history turns 和 live session view model。
- permission response 仍通过正确的 `sessioRuntimeSessionId` 调用。
- 没有引入共享滚动状态导致的自动滚动串扰。

### Phase 3: History + Live 多 lane 展示

目标：多个 sessions 可以同时展示 history 和 live，且容器不串流。

执行内容：

- 抽出 `threadReplayView.ts`，复用 `ThreadPage` 当前 replay grouping helper。
- 基于 `getThreadReplay` 构建 `ThreadSessionLane[]`，沿用后端 `(agent, sessionId)` 聚合结果，并保留多个 source badge。
- 每个 lane 使用稳定 `laneId = agent + ":" + sessionId + ":" + groupKey`。
- 每个 lane 单独调用 `getSessionHistory(agent, filePath, sessionId)`；无 `filePath` 时显示 pending/missing。
- 若 `runtimeSessionAliases[agent:sessionId]` 映射到 `liveState.sessions[id]`，则把 live turns merge 到该 lane。
- 若本页 pending session 尚未获得真实 session id，则用 `pendingNewChats[sessioRuntimeSessionId]` 创建 pending lane；拿到真实 id 后再和 replay/history lane 合并。
- `history + live` 去重使用现有 history/live merge helper，避免 indexed turns 和 live turns 重复。

验收：

- 多个 agent 同时 streaming 时，各自只更新自己的 panel。
- history + live 同 session 不重复显示已 indexed turns。
- 一个 session 多个 source 时只显示一个 lane，并保留多个 source badge。
- Codex 输出很长内容时，不影响其他 lane 的滚动位置。
- history 文件缺失、partial session、astra_internal reference-only session 都有明确状态。

### Phase 4: 用户级 Thread Session 单起

目标：用户可以在新页面里对当前 thread 发起一个普通 thread session，且不会自动跳离本页。

执行内容：

- 在页面底部放置 composer，默认单 agent 选择，复用 `useChatComposer` 或抽取其可复用部分。
- 发送时注入 thread context：
  - thread goal / kind / description。
  - process 当前 stage、teamwork assistants 摘要，或 brainstorm/debate agent participants 摘要。
  - 最近 plan round/task 摘要。
- pending session 写入：
  - `threadLink: { threadId, stageId: null }`。
  - process 若用户选定 stage，则 `stageId` 使用该 stage。
  - `suppressAutoSelect: true`，避免新 session 创建后自动跳离 multi-session page。
  - `origin: "thread_multi_session"`，便于 debug 和后续筛选。
- `PendingNewChatSession` 增加 `suppressAutoSelect` 和 `origin`。
- `usePendingNewChats` 增加完整 suppress 支持：
  - 仍创建 pending session。
  - 仍保存 history/work snapshot。
  - 仍 link thread/stage。
  - 仍更新 runtime alias 和 session list。
  - 不 `setSelected`。
  - 不 `setSelectedThread(null)`。
  - 不 `setDetailMode("chat")`。
  - 不写入会触发 `useSelectedSessionSync` 自动切走的 `pendingSelectSession`。

验收：

- 在新页面发送用户级 thread session 后，新的 pending/live lane 立即出现。
- session 获得真实 agent session id 后仍留在当前页面。
- reload 后该 session 能通过 `getThreadReplay` 回到 thread。
- "Open full chat" 能打开单 session `ChatPage`。
- 原 `NewChatPage` / `ThreadChatPage` 没有受 `suppressAutoSelect` 新字段影响。

### Phase 5: 继续运行未完成 Thread

目标：新页面能根据 thread kind 和当前状态触发或观察继续执行。

执行内容：

- `teamwork`：
  - 接入现有 `createAstraRun(threadId, prompt?)` / `cancelAstraRun(runId)`。
  - 显示 active run、plan rounds、running/planned tasks。
  - running task 对应 sessions 进入 lanes。
- `brainstorm`：
  - 接入现有 Astra run。
  - UI 文案体现 shared-board brainstorm：divergence opinions、synthesis、diagnostics。
  - 使用 plan round/task sessions 进入 lanes。
- `debate`：
  - 接入现有 Astra run。
  - UI 文案体现 isolated lane / cross-check：lane artifacts、visibility policy、convergence diagnostics。
  - 按 round + lane/task 分组展示 sessions。
- `process`：
  - 接入现有 `createAstraRun(threadId, prompt?)` / `cancelAstraRun(runId)`，但 run 必须走 deterministic process backend。
  - 可继续提供 "Run selected stage task" 作为显式单 stage 手动任务。
  - v1 使用 manual plan round/task + runtime session 执行：创建 `PlanRound(source="manual")`，task 绑定 `threadStageId`，保存 stage/assistant/agent snapshot，启动 session 后 `linkPlanTaskSession(role="runtime")`。
  - 明确 task 生命周期：
    - session 启动成功后 task 标记 `running`。
    - session 正常结束后 task 标记 `completed`，并写入 result summary。
    - session failed/cancelled 后 task 标记 `failed` 或 `cancelled`。
    - permission pending 不改变 task terminal status。
  - stage status 仍通过现有 stage state API 更新，不由 plan task 隐式推进。

验收：

- teamwork/brainstorm/debate 未完成 run 可在页面继续观察、取消、刷新。
- teamwork/brainstorm/debate 的 plan task session 出现在对应 lane。
- process 可从页面启动或恢复 deterministic run，也可启动一个手动 stage task 并进入对应 live lane。
- running tasks reload 后能按 plan task 状态恢复展示。
- process 不误报支持 teamwork LLM orchestration。

### Phase 6: 清理与回归

目标：沉淀共享模块，降低长期维护成本。

执行内容：

- `ThreadPage` 和 `ThreadMultiSessionChatPage` 共用 replay grouping helper。
- `ChatPage` 和 `ThreadMultiSessionChatPage` 共用 transcript renderer。
- 清理临时兼容代码，确保 renderer 的 props 表达清晰。
- 为 lane 构建、pending suppress、grouping、history/live merge 增加测试。

验收：

- `ChatPage` 原有单 session 行为不变。
- 新页面和 `ChatPage` 对同一个 session 的 markdown/tool/permission 展示一致。
- 多 lane 页面没有使用同一个 scroll cache key。
- `ThreadPage` replay sessions 展示不丢 source badges。

## API / 类型变更

- `PendingNewChatSession` 新增：

```ts
interface PendingNewChatSession {
  suppressAutoSelect?: boolean;
  origin?: "new_chat" | "thread_chat" | "thread_multi_session";
}
```

- `DetailMode` 或等价 route 状态新增 `threadMultiSessionChat`。
- 新增前端-only 类型：

```ts
type ThreadSessionLaneStatus =
  | "history"
  | "live"
  | "pending"
  | "missing"
  | "failed";
```

- process stage task v1 不新增后端 schema，复用：
  - `createPlanRound`
  - `updatePlanTaskStatus`
  - `linkPlanTaskSession`
  - `completePlanTaskAndStartNext` 仅在需要 sequential round 推进时使用。
- 不改变 `getThreadReplay` wire shape；v1 直接使用现有 source labels/snapshots。

## 测试计划

Unit：

- replay sessions 按 thread kind 分组。
- `(agent, sessionId)` 去重但保留多个 sources。
- pending session 能在真实 id 出现前生成 lane。
- live alias 能正确映射到 lane。
- `suppressAutoSelect` 时 pending session 不切走页面，也不触发 `pendingSelectSession`。
- process manual task status 能根据 live session terminal 状态更新。

Frontend integration：

- 同 thread 多个 history sessions 同屏展示。
- 两个 live sessions 同时 streaming，各自 panel 更新。
- permission request 只出现在对应 lane，响应时使用正确 runtime session id。
- process stage task 创建后出现在 replay/lane。
- teamwork/brainstorm/debate Astra run 产生 plan task session 后出现在对应 lane。

Regression：

- 原 `ChatPage` 单 session 发送、resume、permission、tool 展示不变。
- 原 `ThreadChatPage` 仍可单独使用。
- 原 `ThreadPage` replay sessions 展示不丢 source badges。
- 原 `NewChatPage` 创建 session 后仍自动跳转到新 session。

Manual：

- `process`、`teamwork`、`brainstorm`、`debate` 四种 thread 都打开新页面。
- reload 后 history/live/pending 状态合理。
- mobile 下 lanes 用 tabs/stack 切换不混流，desktop 下多列不重叠。
- 长输出、tool group、permission pending、history file missing 都能读懂状态。

## 假设与默认

- 新页面作为独立入口，不替换现有 `ThreadChatPage`。
- 默认布局是分容器 lanes，不提供全局混排作为主视图。
- v1 支持用户级 thread session 单起。
- v1 支持 teamwork/brainstorm/debate 通过现有 Astra run 继续执行或观察。
- v1 支持 process 通过 manual plan round/task 执行 stage task。
- transcript renderer 抽取是多 lane live 的前置条件，不放到最后清理阶段。
