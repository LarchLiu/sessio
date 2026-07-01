# Canvas Workflow 卡片 Live Turn 动态更新方案

> 本文是 **workflow 卡片** 随 live turn 动态更新的具体实施方案。通用化的插件框架
> 见 [canvas-live-plugin-architecture-plan.md](./canvas-live-plugin-architecture-plan.md);
> 本文的 P0–P3 是"尽快让 workflow 卡片 live 更新"的最短路径,**不为单张卡片先建通用框架**,
> 通用框架列为 P4,对应插件文档 §10 的起步阶段。
>
> 本文经代码核对后按下述 P0–P4 路线重写(取代早期"改 props + `doc.updateBlock` 写 diff"的
> 初稿)。关键修正:live 运行态**默认走 React overlay、不写 Yjs**;首版事件源是
> `agent-runtime-turn-snapshot` 而非 turn 级 delta。

## 目标

让 canvas 上的 `sessio:workflow-card` 块能基于 live turn message 实时更新自身的
workflow 结构(`stages`、`assistants`、运行态、rollup),而不再只是创建时拉一次静态快照。

## 现状(已确认)

- **卡片本体** `sessio:workflow-card` 是 BlockSuite edgeless 块
  (`src/lib/blocksuite/blocks/workflow-card/model.ts`)。props 已含
  `threadId`、`threadStageId`、`executionState`、`lastRunId`、
  `workflowSnapshotJson`、`workflowSummaryMarkdown`、`status`。
  stages/assistants 的数据**已被序列化进 `workflowSnapshotJson`**,只是
  `WorkflowCardHost`(`host.tsx`)目前只渲染 `workflowSummaryMarkdown` 文本,
  没有把 stages/assistants 结构化画出来。
- **数据源** 是 `getThreadWorkSnapshot(agent, sessionId)` → `ThreadWorkSnapshot`,
  其中 `stages[]` 每项带 `status`、`assistants`、`issues`、`sessionRefs`,还有 `rollup`。
  卡片创建时(`addWorkflowCard`,`BlockSuiteCanvasHost.tsx:1532`)只拉一次,是**静态快照**。
- **Live 事件源(关键约束)**:两条 tauri 通道,但前端可用性不同 ——
  - `agent-runtime-turn-snapshot`(整份 `LiveRuntimeSession`,含 `turns[]`):
    经 `useRuntimeEventSubscription`(hooks/useRuntimeEventSubscription.ts:112-116)
    进入前端,有 160ms 批处理 + 终态(completed/failed/cancelled)立即 flush。
    **这是首版唯一可靠的 live 源。**
  - `agent-runtime-event`(`turnStarted` / `textDelta` / `toolStarted` /
    `turnCompleted` …):`shouldDispatchRuntimeEvent`
    (useRuntimeEventSubscription.ts:139-141)**当前只放行 `sessionStarted` /
    `sessionEnded`**,turn 级 delta 不进 `dispatchLiveRuntimeEvent`,仅用于算未读。
    要消费 delta 须先扩流该白名单(或另起 `listen`,注意重复订阅)。
- **写块与重绘**:`runWorkflowBlock`(`BlockSuiteCanvasHost.tsx:1584`)用
  `doc.updateBlock(model, {...})` 改 `executionState` / `status`;lit 组件靠
  **`component.ts:28-37` 的 `rerenderToken`** 触发重绘(注意:是 `component.ts`,
  `view.ts` 只做 `FlavourExtension` + `BlockViewExtension` 注册,没有 token)。
- **写块的隐藏代价**:`doc.slots.blockUpdated` **无条件**触发 `scheduleAutosave`
  (`BlockSuiteCanvasHost.tsx:1270-1276`),而 `bridge.updateBlock` 内部走
  `doc.updateBlock`。因此把高频 live delta 写进块 props 会持续 churn canvas draft
  autosave —— 这是首版必须避开的坑。

结论:缺的不是写通道,而是 **① 卡片先把 `workflowSnapshotJson` 结构化画出来**,
**② 一条从 turn snapshot 到"瞬时运行态"的投影,且这个运行态不落 Yjs**。

## 核心决定:持久态与瞬时态分离

| | 存储 | 谁写 | undo / autosave | 内容 |
|---|---|---|---|---|
| **持久态** | 块 prop `workflowSnapshotJson` | 仅终态对账 | 进 undo / 触发 autosave | 权威 `ThreadWorkSnapshot` |
| **瞬时运行态** | 块外 overlay store(`Map<blockId, {overlay, revision}>`) | 每次 snapshot 投影 | 都不触发 | stage active、assistant 活跃点、当前动作文案 |

`WorkflowCardHost` 渲染时**合并两者**:以 `workflowSnapshotJson` 为底,overlay 覆盖运行态字段。
这样高频跳动只走 React portal 重渲,不碰 CRDT;只有权威终态(低频)才落进块 prop。
注意:workflow card 是 Lit block 内用 `reactToLit` 挂 React portal,普通
`BlockSuiteCanvasHost` React state 更新**不会自动重绘已挂载的 Lit block**。P2 必须提供
按 `blockId` 订阅的 workflow overlay store,由 `workflow-card/component.ts` 订阅后
`requestUpdate()`,并把 overlay revision 放进 `rerenderToken`。

> 单一持久 payload:**不新增 `stagesJson` / `assistantsJson` prop**。显示数据一律从
> `workflowSnapshotJson` 派生。否则要同步 model getter、host props、rerenderToken 与
> `workflowCardModelToCanvasBlock`(blockSuiteInterop.ts:75)的 metadata,得不偿失。

## Agent work-state skill 与 workflow 编排边界

workflow live card 的数据稳定性不能依赖 assistant 最终回复里"说出 JSON"。正确分层是:

1. **agent 通过受控 tool/CLI 写结构化状态**。当前入口是
   `~/.sessio/bin/sessio ... --json`;打包后通过 bundled resource skill 注入给 agent,
   方式与 computer-use skill 一致,不能依赖开发仓库里的 `.agents/skills` 目录。
2. **Sessio store / Tauri API 是权威状态源**。Canvas 只读 `ThreadWorkSnapshot` +
   runtime overlay;高频 live overlay 不写 Yjs,低频终态对账才写 block prop。
3. **assistant prose 只是解释层**。不得从自然语言回答反推 stage status / issues /
   workflow 结构;否则同一轮对话会同时存在"看起来完成"和"结构化状态未完成"两套真相。

当前 bundled `sessio-work-state` skill 的能力边界是**创建/编排/更新 thread-stage workflow**:

- 创建/维护 thread:`sessio thread create/list/show/update/set-stage`
- 编排 session:`sessio thread link-session/unlink-session`
- 发现/创建/维护 stage:`sessio stage catalog/add/list/show/configure`
- 编排 stage session:`sessio stage link-session/unlink-session`
- 写进度:`sessio stage set-status`,`sessio stage update`
- 结构化问题:`sessio stage issue add/list/set`
- 不提供 destructive delete;stage 移出 active plan 用 `configure --enabled false` 或
  status `skipped`,issue 用 `set --status dismissed`

### 普通 session 自举 workflow

普通 session 对话现在也可以在需要时创建 thread,配置 workflow,并把自己或后续子会话
链接到 thread/stage。agent-safe orchestration surface 是:

```bash
~/.sessio/bin/sessio thread create \
  --project <projectPathOrId> \
  --goal "..." \
  --description "..." \
  --kind process \
  --json

~/.sessio/bin/sessio stage catalog \
  --project <projectPathOrId> \
  --json

~/.sessio/bin/sessio stage add \
  --thread-id <threadId> \
  --stage-id <projectStageId> \
  --json

~/.sessio/bin/sessio thread link-session \
  --thread-id <threadId> \
  --agent <codex|claude|opencode|pi> \
  --session-id <sessionId> \
  --json

~/.sessio/bin/sessio stage link-session \
  --stage-id <threadStageId> \
  --agent <codex|claude|opencode|pi> \
  --session-id <sessionId> \
  --json
```

`--project` 接受项目路径或 project id;若 agent 只知道 cwd,可使用
`thread create --project <cwd>` 解析到 Sessio project。创建类命令返回完整
`ThreadInfo` / `StageInfo` JSON,其中包含 `threadId` / `threadStageId`,让后续命令可以
继续编排。agent-safe CLI 仍避免物理删除;禁用/跳过用 `enabled=false` 或 status 表达。

中期更理想的是把上述 CLI 能力同步成 Sessio workflow MCP tools,例如:

- `workflow_create_thread`
- `workflow_add_stage`
- `workflow_link_session`
- `workflow_get_snapshot`
- `workflow_update_stage_state`
- `workflow_add_issue`

CLI 是短期稳定落点,MCP 是更好的 agent tool UX。无论走 CLI 还是 MCP,Canvas 的消费面
不变:仍然只读 `ThreadWorkSnapshot` 与 live overlay。

## 分阶段路线(P0–P4)

### P0 — 打通 prop wiring(前置,无行为变化)

`ChatCanvasView`(ChatCanvasView.tsx:11-28)与 `BlockSuiteCanvasHost` 目前**没有**
`liveState` / `runtimeSessionAliases` prop;`ChatPage`(ChatPage.tsx:1808 附近)渲染
`ChatCanvasView` 时也没下传。而 `liveState` / `runtimeSessionAliases` 在 `ChatPage`
内部已有(ChatPage.tsx:184、336、340)。

```
ChatPage(已有 liveState / runtimeSessionAliases)
  → ChatCanvasView(新增 prop 透传)
    → BlockSuiteCanvasHost(新增 prop → 供后续投影使用)
```

这是一切 live 更新的前提,独立可测(传下去不用即无副作用),先落地。

### P1 — 卡片结构化展示 `workflowSnapshotJson`(纯渲染,可单测)

只改渲染,不接 live,先把"能看见 stages/assistants"跑通:

1. `blocks/workflow-card/host.tsx`(`WorkflowCardHost`):新增 prop
   `workflowSnapshotJson`,解析
   `workflowSnapshotJson` → 渲染 stage 列表(stage 名 + status 徽标 + issues 计数)
   与每个 stage 下的 assistant(名字/颜色/头像点),取代当前只显示 summary 文本。
   派生逻辑抽成纯函数(如 `parseWorkflowSnapshot`)配单测,沿用
   `test/threadSnapshot.test.ts` 习惯。
2. `blocks/workflow-card/component.ts`:必须把 `model.workflowSnapshotJson` 传给
   `WorkflowCardHost`,并把 `workflowSnapshotJson` 加进 `rerenderToken`。当前 token
   还没包含它,否则终态快照变化不会重绘。

到此卡片是"结构化的静态快照",已比现状好用,且零 live 风险。

### P2 — live 运行态 overlay(不写 Yjs)

用 P0 传下来的 `liveState` 驱动瞬时态:

1. **关联层**(见下节 §关联层规格):由 `runtimeSessionAliases` + 每张 workflow 卡片
   自己的 `workflowSnapshotJson` 中的 session refs 建
   `sessioRuntimeSessionId ↔ {agent, childSessionId, threadId, stageId}` 双向索引,
   把 live turn 路由到某 `threadId` 下的卡片 blockId。
2. **投影(纯函数)**:`liveState.sessions` 里命中本卡 thread 的 turn →
   派生 overlay(该 stage `in_progress`、对应 assistant `active`、可选"当前动作"文案)。
   ```ts
   interface WorkflowOverlayCardContext {
     blockId: string;
     threadId: string;
     threadStageId: string | null;
   }

   projectLiveOverlay(card, prevSnapshot, liveTurns, mapping) => WorkflowOverlay
   ```
   `card.threadStageId` 决定投影粒度:stage 卡只投影该 stage / sessionRefs 命中的运行态,
   thread 总卡才聚合 thread rollup。
3. **写 overlay 而非写块**:结果存进 `BlockSuiteCanvasHost` 持有的
   `WorkflowOverlayStore`(`Map<blockId, { overlay, revision }>`),并通过
   `portalBridge.workflowOverlay` 暴露给 workflow block。store 至少支持:
   ```ts
   get(blockId: string): { overlay: WorkflowOverlay; revision: number } | null;
   set(blockId: string, overlay: WorkflowOverlay): void; // revision +1 并通知该 block
   delete(blockId: string): void;                        // revision +1 并通知该 block
   clear(): void;                                        // 清空并通知所有已订阅 block
   subscribe(blockId: string, fn: () => void): () => void;
   ```
   `workflow-card/component.ts` 在 `connectedCallback`/`willUpdate` 订阅自己的
   `blockId`,overlay revision 变化时 `requestUpdate()`;`renderBlock()` 读取 overlay,
   将其传入 `WorkflowCardHost`,并把 `overlay.revision` 加进 `rerenderToken`。
   高频更新只触发 React portal 重渲,**不 `updateBlock`、不进 undo、不触发 autosave**。
   `portalBridge.workflowOverlay` 的 store identity 要么在 workflow block 首次 render 前稳定设置好,
   要么 `portalBridge` 必须提供 bridge/store changed 通知,让已挂载的 workflow block
   `requestUpdate()` 后重新订阅;否则卡片可能停在旧 store 或 fallback。
4. 复用订阅层 160ms 批处理,每帧只取最后一次投影。

### P3 — 终态权威对账(低频写块)

真值仍以后端为准:

1. **触发去重**:检测某 turn 的 `status` 从非终态**变为** `completed` / `failed` /
   `cancelled`(在 snapshot 流里对比上一次记录的 turn status),仅此**边沿**触发一次
   `getThreadWorkSnapshot(agent, childSessionId)`。**不要**每收到含已完成 turn 的
   snapshot 就重复拉。fetch 去重 key 用
   `{sessioRuntimeSessionId}:{turnId}:{terminalStatus}`,同一 thread 多张卡片 fan-out 时
   只拉一次 snapshot,再把结果写回对应 `blockIds`。
2. 拉回的权威 `ThreadWorkSnapshot` 与卡片现有 `workflowSnapshotJson` diff,有变化才
   在 `BlockSuiteCanvasHost` 内低频 `doc.updateBlock(model, { workflowSnapshotJson,
   workflowSummaryMarkdown, executionState, status })`(或 host 内部等价 helper)。
   `workflowSummaryMarkdown` 必须用 `workflowSnapshotToMarkdown(snapshot)` 同步更新,避免
   旧 summary / interop metadata 继续显示 stale 内容。diff 范围是 canonical snapshot string +
   derived summary + `executionState/status`,不要只比较 snapshot object。canonical snapshot
   string 首版可用同一个稳定序列化 helper 生成;是否包含 `capturedAt` 要明确。若
   `capturedAt` 只代表拉取时间、会导致无意义抖动,应在 diff 时排除或归一化。
   这是**低频**写,autosave / undo 代价可接受。
3. 写回后清掉该卡对应的 overlay(权威态已覆盖瞬时态)。
4. 拉取失败:保留上一次权威态 + 继续 overlay 跳动,不清零。

### P4 — (可选)抽象为 live-card 插件框架

当出现第二、三种 live 卡片(工具活动、文件编辑汇总)时,再把 P1–P3 的
关联/投影/overlay 抽成通用插件框架,workflow 成为第一个插件。详见
[canvas-live-plugin-architecture-plan.md](./canvas-live-plugin-architecture-plan.md)。
**单张卡片阶段不做这一步。**

## 关联层规格:`SessionThreadStageMap`

事件/快照只带 `sessioRuntimeSessionId`(+ `turnId`),卡片按 `threadId` /
`threadStageId` 归属。需**双向索引**,不能只用裸 `sessionId`——它会跨 agent / 子会话
碰撞,且 `getThreadWorkSnapshot` 需要 child **agent + session**,不止 `threadId`。

```ts
interface SessionThreadStageMap {
  // 正向:运行时会话 → 归属实体(把 live turn 路由到卡片)
  bySessioRuntimeId: Map<string, {
    agent: Agent;
    childSessionId: string;          // 拉 snapshot 用
    threadId: string | null;
    stageId: string | null;
    assistantId: string | null;
  }>;
  // 反向:thread → 该 thread 下所有卡片 blockId(fan-out)
  blockIdsByThread: Map<string, Set<string>>;
}
```

三个来源合成:

- `runtimeSessionAliases` 是 `{agent}:{sessionId} → sessioRuntimeSessionId`
  (ChatPage.tsx:398、useRuntimeEventSubscription.ts:34),需**反转**成
  `sessioRuntimeSessionId → {agent, childSessionId}`。
- 每张 workflow 卡片的 `workflowSnapshotJson` 提供 `threadId` 与 session refs。收集时不要
  只看 `stages[].sessionRefs`;还要合并 `threadSessionRefs` 与
  `detailRefs.sessionRefs`,兼容 thread-level session 与旧/补充格式。
- `ThreadWorkSnapshotStage.sessionRefs[]`(api.ts:2250、2253)提供
  `{agent, sessionId} → {threadId, stageId}`,补齐 stage 归属字段。
- `blockIdsByThread` 在 canvas 挂载时遍历 doc 里所有 `sessio:workflow-card` 按
  `threadId` 归组构建;后续在 `doc.slots.blockUpdated` 的 add/delete、workflow card
  `threadId` / `threadStageId` / `workflowSnapshotJson` 变化时重建或增量更新。不要只依赖
  `syncCanvasBlocks`,它是持久化同步语义,不是内存索引唯一触发点。

未命中(session 不属任何已知 thread/卡片)即丢弃,避免无关刷新。
投影 fan-out 时还要看卡片自己的 `threadStageId`:若卡片有 stage id,只显示该 stage
或该 stage sessionRefs 命中的 live 状态;`threadStageId` 为空的卡才显示 thread rollup,
避免同一 thread 下的总卡和 stage 卡都展示过宽的 live 状态。

## 边界与生命周期

- 卡片创建/删除时增删 `blockIdsByThread`;canvas 卸载时清订阅与 overlay。
- 同一 thread 多张卡片:反向索引是 `Set<blockId>`,一次投影 fan-out 到全部。
- 卡片 `threadId` 为空(未链接):跳过,保持现有 idle 行为。
- overlay 只在内存:canvas 重载后由后续 snapshot 自然重建,无需持久化。
- `workflowOverlayStore` 生命周期跟随 `BlockSuiteCanvasHost`;切换 session/doc 时清空 store,
  并确保 workflow block 释放旧订阅,重新读取当前 bridge/store identity。
- P1 可独立落地:此时 `WorkflowCardHost` 只需要 `workflowSnapshotJson`。`WorkflowOverlay`
  类型、`overlay` prop、store 与订阅逻辑从 P2 引入,避免 P1 被 live 运行态耦合。

## 改动点清单(按阶段)

| 阶段 | 文件 | 改动 |
|------|------|------|
| P0 | `pages/ChatPage.tsx`、`components/ChatCanvasView.tsx`、`components/blocksuite/BlockSuiteCanvasHost.tsx` | 透传 `liveState` / `runtimeSessionAliases` prop |
| P1 | `blocks/workflow-card/host.tsx` | 解析 `workflowSnapshotJson`,渲染 stages/assistants;派生纯函数 + 单测 |
| P1 | `blocks/workflow-card/component.ts` | 传递 `workflowSnapshotJson`;补 `rerenderToken` |
| P2 | 新增 `lib/blocksuite/workflowLiveProjection.ts` | `projectLiveOverlay` + `SessionThreadStageMap` 构建,配单测 |
| P2 | `lib/blocksuite/portalBridge.ts` | 新增 `workflowOverlay` 字段;必要时提供 bridge/store changed 通知 |
| P2 | `components/blocksuite/BlockSuiteCanvasHost.tsx` | workflow overlay store + block 订阅重绘 + 160ms 节流投影 |
| P3 | `components/blocksuite/BlockSuiteCanvasHost.tsx` | turn 终态边沿去重 → 拉 snapshot diff → 低频 `doc.updateBlock` |

## 落地顺序

P0 → P1 先行(零/低风险,可单测),让"卡片能看见结构化 workflow"落地;再 P2 接 overlay
运行态;最后 P3 终态对账。P4 视是否出现更多 live 卡片再评估。
