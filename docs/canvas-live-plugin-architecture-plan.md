# Canvas Live 插件化架构设计

> **状态:中长期架构草案,非近期实施方案。**
> 本文记录一个方向性判断——*schema 不能热插拔,故用通用 `live-card` 承载运行时插件*——
> 以及围绕它的契约设计。但作为落地方案它**范围过大**,doc 模式、外部插件加载等
> 仍需在最小 workflow 链路跑稳后再验证。
>
> **近期不要按本文实施。** 让 workflow 卡片尽快 live 更新的最小闭环见
> [canvas-workflow-live-update-plan.md](./canvas-workflow-live-update-plan.md)
> 的 P0–P3(现有 `workflow-card` + liveState prop wiring + React overlay + 终态 snapshot 写回)。
> 那条链路跑稳后,再回头评估是否把本文的 `live-card` + registry 抽象出来(= 那份文档的 P4)。
>
> ---
>
> 目标:定义一套可热插拔的插件架构,把 **live turn message** 解析成结构化数据,
> 再声明它在 canvas 中的显示方式。新增/删除一种 live message 类型,
> 理想情况下**不触碰 BlockSuite schema、不改渲染主流程**,只在运行时 registry 增删插件。
>
> **范围界定(review 后收紧):本文只讨论 canvas block 的 live 投影**,
> *不*涵盖聊天 transcript(`acpRenderItems.ts`)的渲染重构 —— 后者是独立议题,见 §1.1。

---

## 1. 现状与约束(必须先对齐)

设计前先确认三条来自现有代码的硬事实,它们直接决定热插拔的边界。

### 1.1 两个不同的 "message → 视图",不要混为一谈

有两处 "message → 视图" 的硬编码,它们**属于不同子系统,应作为两个独立议题**:

1. **聊天 transcript 渲染** —— `src/acpRenderItems.ts` 的
   `acpViewModelToRenderItems`(acpRenderItems.ts:139)用一长串 `if (block.kind === ...)`
   把一个 turn 拆成 `AcpRenderItem[]`,渲染在聊天流里。

   ```
   AcpRenderItem =
     | turnStatus | workingIndicator | block | tool | toolGroup | permission | error
   ```

2. **canvas block 投影** —— 把 live turn 投影成 canvas 上的卡片(本文主题)。

> ⚠️ **本文只讨论第 2 项。** transcript registry 是另一个议题:它渲染进聊天流、
> 不涉及 BlockSuite doc、没有 autosave/undo 约束,生命周期也不同。
> 把 `acpRenderItems` 的重构塞进 canvas live 架构会让首版无谓地背上 transcript 重构包袱。
> 若将来要做 transcript 插件化,应另立文档,**顶多**与本文共享 `ingest` 解析层的思路,
> 不共享 sink / 承载块 / reconcile。

### 1.2 BlockSuite 块注册是声明式、且构建期完成的

`src/lib/blocksuite/specs.ts` 用三件套注册自定义块:

- `FlavourExtension("sessio:workflow-card")`
- `BlockViewExtension("sessio:workflow-card", literal\`sessio-edgeless-workflow-card\`)`
- `BlockSchemaExtension(WorkflowCardBlockSchema)`

这些扩展在 **editor / doc 初始化时一次性注册**(specs.ts:52-77)。

### 1.3 CRDT 硬约束(决定性)

Yjs 文档要反序列化一个块,**必须在 doc 创建前就知道该块的 schema**。
因此"运行时动态新增一个块 flavour"不可行。

> ✅ 结论:**解析层、投影层、渲染逻辑可以热插拔;块 schema 不能。**
> 架构的核心任务就是用"通用承载块"把 schema 约束挡在插件契约之外。

### 1.4 前端当前拿不到 turn 级增量事件(决定首版事件源)

`useRuntimeEventSubscription.ts:139-141` 的 `shouldDispatchRuntimeEvent`
**只放行 `sessionStarted` / `sessionEnded`**:

```ts
function shouldDispatchRuntimeEvent(kind: string): boolean {
  return kind === "sessionStarted" || kind === "sessionEnded";
}
```

`turnStarted` / `textDelta` / `toolStarted` / `turnCompleted` 等 turn 级事件目前
**不会进入 `dispatchLiveRuntimeEvent`**,只被用来计算未读(:91-102)。
前端真正可消费的 live 数据是 `agent-runtime-turn-snapshot`(整份
`LiveRuntimeSession`,含 160ms 批处理,:112-116)。

> ✅ 约束:`IngestContext` 首版应以 **`snapshot` 为主源**;`IngestContext.event`
> 只有在扩流 `shouldDispatchRuntimeEvent`(或另起独立 `listen`,注意重复订阅)
> 之后才可靠。插件的 `ingest` 不应假设能拿到 `textDelta` 级增量。

### 1.5 canvas 尚无 live 状态的 prop 通路(P0 前置)

`ChatCanvasView`(ChatCanvasView.tsx:11-28)没有 `liveState` /
`runtimeSessionAliases` prop,`ChatPage`(ChatPage.tsx:1808 附近)渲染它时也没有下传。
而 `liveState` / `runtimeSessionAliases` 在 `ChatPage` 内部是有的
(ChatPage.tsx:184、336、340)。所以管线要在 canvas 侧运行,必须先补这段 wiring:

```
ChatPage(已有 liveState/runtimeSessionAliases)
  → ChatCanvasView(新增 prop 透传)
    → BlockSuiteCanvasHost(新增 prop → 喂给 pipeline)
```

这是一切 live 管线的前置条件,与插件契约无关,应最先落地。

---

## 2. 关注点分层

一条 "live message → canvas 显示" 的链条是三个正交职责。一个插件 = 这三者的绑定单元。

```
raw live event ──①ingest──▶ 规范化 message ──②project──▶ canvas mutation ──③render──▶ 视图
   (两条 tauri 通道)          (结构化 payload)         (upsert/patch/remove)      (React / lit)
```

| 层 | 职责 | 纯度 | 可热插拔 |
|----|------|------|----------|
| ① ingest | `AgentRuntimeEvent` / 快照 → 本插件的结构化 payload | 纯函数 | ✅ 完全 |
| ② project | payload + 上一份 payload → 对卡片的增量意图 | 纯函数 | ✅ 完全 |
| ③ render | payload → 视图组件(在承载块内被调用) | 组件 | ✅(靠承载块) |
| — schema | 块的 CRDT 定义 | — | ❌ 构建期 |

事件来源(沿用现状):

- `agent-runtime-turn-snapshot`:整份 turn 快照,经 `useRuntimeEventSubscription`
  (hooks/useRuntimeEventSubscription.ts)160ms 批处理进入前端。**首版主源。**
- `agent-runtime-event`:增量事件流(`textDelta` / `toolStarted` / `sessionUpdate` /
  `turnCompleted` …,见 api.ts:1327-1406)。**当前受限**——`shouldDispatchRuntimeEvent`
  只放行 session 级事件(见 §1.4),turn 级增量暂不可用,需先扩流才能作为 ingest 输入。

---

## 3. 核心决定:通用承载块 `sessio:live-card`

### 3.0 overlay 宿主定死:live-card 作 **anchor block**,payload 走 overlay(不进 props)

这是 review 暴露的最大架构洞,必须先定死,否则"默认 overlay"与"承载块渲染"互相矛盾:
没有块就没有 `blockId` / `xywh` / selection / zoom placement / 组件挂载点;
但把 live payload 写进块 props 又会 churn autosave(§5.3)。

**定稿方案(anchor block,非独立 overlay layer):**

- `live-card` 块**只作锚点**:承载 `blockId` / `xywh` / `pluginId` / `cardKey`,
  提供挂载点、布局、命中、缩放。它**只在建卡 / 移动 / 删卡时写 Yjs**——低频,
  autosave 代价可接受。
- **live payload 不进块 props**,全程走 overlay store(`Map<blockId, payload>`)。
  高频 live 更新只改 React store,**不发 `blockUpdated`、不触发 autosave**(§5.3)。
- overlay store 必须支持**按 blockId 订阅**。`live-card` 组件在 `connectedCallback`
  订阅自己的 `blockId`,收到 overlay revision 变化时 `requestUpdate()`;否则块外
  React store 更新不会触发 Lit 的 `renderBlock()`、`rerenderToken` 也不会重新计算。
- `live-card` 也必须订阅 **registry 变化**。未知 `pluginId` 先渲染 fallback 后,若插件稍后
  注册/注销,仅靠 overlay revision 不会触发重绘;需要 `liveRegistry.subscribe()` 或 bridge-level
  registry revision 通知 `requestUpdate()`。
- 订阅要能处理 bridge/store 生命周期变化:若组件 connect 时 bridge 尚未设置,或 session
  切换导致 `liveOverlay` 实例替换,组件必须在 `renderBlock`/`updated` 检测 store identity,
  重新订阅当前 store 并释放旧订阅。不要假设 `connectedCallback` 一次订阅永久有效。
- 承载块渲染时,**overlay store[blockId] 优先,回落到 `model.payloadJson`**
  (后者仅在 doc/pinned 模式或重载后才有值)。
- **复用 BlockSuite 既有机制**:placement / viewport transform / selection /
  `getRenderingRect` 的屏幕像素缩放(与 `workflow-card` 的"清晰 DOM 卡片、
  非真实缩放"策略一致,component.ts:87-103),**不自建 overlay layer** 重新处理
  viewport / z-index / pointer-events / 框选。

> 为何不选"独立 canvas overlay layer":那样要自己重造 viewport transform、命中、
> 框选期 pointer-events、与 edgeless 选区的协调,成本高且易与现有 canvas 行为漂移。
> anchor block 用一次性低频写块换来全部这些机制免费复用,是首版更小的闭环。

这也修正了落地顺序的依赖(见 §10):首版 overlay **仍需** live-card anchor,
只是 anchor 不承载 live payload。

### 3.1 块定义

不给每种 message 定义独立 flavour(那需要构建期注册、无法热插拔)。
改为定义**唯一一个通用块**,把"是什么"放进 props,渲染时按 `pluginId` 查 registry。
注意:按 §3.0,`payloadJson` 只在 doc/pinned 模式承载数据;overlay 模式下它可为空,
真实 live payload 来自 overlay store。

```ts
// src/lib/blocksuite/blocks/live-card/model.ts —— 唯一需要预注册的动态块
export interface LiveCardBlockProps extends GfxCompatibleProps {
  pluginId: string;    // 由哪个插件渲染
  cardKey: string;     // 稳定身份,reconcile 用(同 key = 同一实体)
  payloadJson: string; // 结构化数据(JSON 序列化)
  revision: number;    // 单调递增,丢弃乱序写入
  title: string;       // 供大纲/无障碍读取的冗余标题
  ownership: "live-anchor" | "live-owned" | "user-pinned";
  placementOwnership: "auto" | "user-positioned";
  sourceRuntimeSessionId: string | null;
  runId: string | null;
  expiresAt: number | null;
}
```

`live-card` 是新增 schema,实现时必须同时定义 **defaults + 兼容策略**,不要只写
TypeScript interface:

- schema `props()` 默认值建议为:`pluginId/cardKey/payloadJson/title = ""`,
  `revision = 0`,`ownership = "live-anchor"`,`placementOwnership = "auto"`,
  `sourceRuntimeSessionId/runId/expiresAt = null`,
  `xywh` 使用插件 `measure` 或通用默认尺寸。
- 旧 snapshot / 未知来源反序列化时,缺失字段必须按上述默认值补齐。特别是
  `sourceRuntimeSessionId` / `runId` 不能强制非空:pinned 卡、手工创建卡、重载恢复卡
  都可能没有运行时来源。
- 后续若 `payloadJson` schema 变化,应在 payload 内带 `schemaVersion`,或由插件提供
  `migratePayload` / `validatePayload`,不要把插件 payload 版本塞进 BlockSuite block schema
  版本里。

> `workflow-card` 现在的 `workflowSnapshotJson` prop(blocks/workflow-card/model.ts:112)
> 已经是这个思路的雏形,只是被写死成单一卡片。`live-card` 把它泛化。

渲染组件内部按 `pluginId` 取渲染器,与 `workflow-card/component.ts` 现有的
`reactToLit` + `rerenderToken` 模式一致(component.ts:26-58):

```ts
// live-card/component.ts(骨架)—— overlay 优先(§3.0)
override connectedCallback() {
  super.connectedCallback();
  this._syncBridgeSubscriptions();
}

private _syncBridgeSubscriptions() {
  const bridge = getBlockSuitePortalBridge();
  const bridgeRevision = getBlockSuitePortalBridgeRevision();
  const ownerId = bridge?.ownerId ?? null;
  const store = bridge?.liveOverlay ?? null;
  const registry = bridge?.liveRegistry ?? null;
  if (
    bridgeRevision === this._bridgeRevision &&
    ownerId === this._bridgeOwnerId &&
    store === this._overlayStore &&
    registry === this._registry
  ) return;
  this._overlayDispose?.();
  this._registryDispose?.();
  this._bridgeDispose?.();
  this._bridgeRevision = bridgeRevision;
  this._bridgeOwnerId = ownerId;
  this._overlayStore = store;
  this._registry = registry;
  const unsubscribeOverlay = store?.subscribe(
    this.model.id,
    () => this.requestUpdate(),
  );
  const unsubscribeRegistry = registry?.subscribe(() => this.requestUpdate());
  const unsubscribeBridge = subscribeBlockSuitePortalBridge(() => this.requestUpdate());
  this._overlayDispose = unsubscribeOverlay ?? null;
  this._registryDispose = unsubscribeRegistry ?? null;
  this._bridgeDispose = unsubscribeBridge;
}

override willUpdate() {
  this._syncBridgeSubscriptions();
}

override disconnectedCallback() {
  this._overlayDispose?.();
  this._registryDispose?.();
  this._bridgeDispose?.();
  this._overlayDispose = null;
  this._registryDispose = null;
  this._bridgeDispose = null;
  this._bridgeRevision = -1;
  this._overlayStore = null;
  this._registry = null;
  this._bridgeOwnerId = null;
  super.disconnectedCallback();
}

override renderBlock() {
  const bridge = getBlockSuitePortalBridge();
  const plugin = bridge?.liveRegistry?.get(this.model.pluginId);
  // overlay store[blockId] 优先;回落到 model.payloadJson(doc/pinned 或重载后才有值)
  const overlay = bridge?.liveOverlay?.get(this.model.id);
  const payload = overlay?.payload ?? safeParse(this.model.payloadJson);
  const revision = overlay?.revision ?? this.model.revision;
  const identity = { pluginId: this.model.pluginId, key: this.model.cardKey };
  const cardId = encodeCardId(identity);
  const rerenderToken = [
    this.selected$.value ? "1" : "0",
    bridge?.ownerId ?? "",
    String(bridge?.liveRegistry?.revision ?? 0),
    this.model.pluginId,
    this.model.cardKey,
    String(revision), // overlay 或 props 的 revision 变化即触发重绘
  ].join("");

  const content = !plugin || !bridge
    ? html`<div class="sessio-live-card-fallback">Unknown live plugin: ${this.model.pluginId}</div>`
    : payload == null
      ? html`<div class="sessio-live-card-fallback">Waiting for live payload.</div>`
      : bridge.reactToLit(
          () => createElement(plugin.render.component, {
            payload,
            cardKey: this.model.cardKey,
            identity,
            cardId,
            host: bridge.liveHost,
          }),
          rerenderToken,
        );

  return html`<div class="sessio-live-card" style=${this.containerStyleMap}>${content}</div>`;
}
```

未知 `pluginId` 时渲染 fallback 占位,而非崩溃 —— 这让插件可以**先出现在数据里、后注册渲染器**,
也让插件卸载后旧卡片安全降级。
无有效 payload 时也必须渲染 fallback/loading,不要把 `null` 传给插件组件;`LiveCardProps<T>.payload`
保持非空,插件只处理自己的合法 payload。
`rerenderToken` 必须包含 bridge owner 与 registry revision(或插件 renderer version):仅
`requestUpdate()` 不足以保证已有 React portal 重建,`reactToLit` 的 token 策略只有 token
变化时才会刷新 portal 内容。插件同 id 热替换、registry 注册/注销、bridge/liveHost 替换
都应改变 token。
`setBlockSuitePortalBridge(...)` 也必须可通知已挂载的 `live-card`:组件可能先在 bridge
为空时渲染 fallback,随后 bridge 才准备好。若没有 `subscribeBlockSuitePortalBridge`
或等价的 bridge-changed event,这些块不会自动重新进入 `willUpdate` / 订阅 overlay。

---

## 4. 插件契约

```ts
// src/lib/canvas/live/types.ts
export interface LiveCanvasPlugin<T = unknown> {
  id: string;

  /** ① 从事件/快照里认领并解析出本插件的 message(增量或整份快照都喂进来)。 */
  ingest(ctx: IngestContext): LiveCanvasMessage<T>[];

  /** ② message + 该 key 的上一份 payload → 对卡片的增量意图(不直接写 doc)。 */
  project(message: LiveCanvasMessage<T>, prev: T | null): CanvasMutation<T>[];

  /** ③ payload → 视图(A 档:承载块内渲染;B 档:指向原生 BlockSuite 组件)。 */
  render: {
    component: React.ComponentType<LiveCardProps<T>>;
    measure?(payload: T): { width: number; height: number };
    /** B 档:插件自带独立 flavour(构建期注册),此处声明其 flavour。 */
    nativeFlavour?: string;
  };

  onMount?(host: LiveCanvasHost): void;
  onUnmount?(): void;
}

export interface LiveCanvasMessage<T> {
  pluginId: string;
  key: string; // 稳定身份;同一实体多次更新须给同一 key
  scope: {
    threadId: string | null;
    stageId: string | null;
    sessionId: string | null;
    turnId: string | null;
  };
  payload: T;
  revision: number; // 单调递增
}

export interface CardIdentity {
  pluginId: string;
  key: string;
}

// sink 内部可序列化索引。实现时要安全转义分隔符,或直接使用 nested Map。
export type CardId = string; // encodeCardId({ pluginId, key })

export type Placement =
  | { kind: "auto" }                          // 交给 placeCanvasNodes 布局
  | { kind: "near"; identity: CardIdentity }  // 靠近另一张卡片
  | { kind: "fixed"; xywh: string };

export type CanvasMutation<T = unknown> =
  | { op: "upsert"; key: string; payload: T; placement?: Placement }
  | { op: "patch"; key: string; merge: (prev: T) => T }  // 插件自带 reducer,见下
  | { op: "remove"; key: string };

export interface IngestContext {
  // 首版主源:传**完整 snapshot event**(含 sequence),不是裸 LiveRuntimeSession。
  // revision 的可靠来源就是 event.sequence —— runtimeChat 已按 session 用
  // sessionSequences 做乱序过滤(runtimeChat.ts:551、574),复用同一序号即可。
  snapshotEvent?: LiveRuntimeTurnSnapshotEvent;  // { session, sequence, timestamp }
  // event 流首版基本拿不到 turn 级 delta(见 §1.4),留作后续扩流。
  event?: AgentRuntimeEvent;
  mapping: SessionThreadStageMap;  // session → thread/stage 关联(见 §7)
}

export interface LiveCardProps<T> {
  payload: T;
  cardKey: string;
  identity: CardIdentity;
  cardId: CardId;
  host: LiveCanvasHost;
}
```

设计要点:

- **`project` 返回意图而非直接落地** —— 落地被 runtime 统一节流、统一走 sink(默认 overlay,见 §5.3),插件永不接触 CRDT。
- **`key` 幂等** —— 同 key 的 upsert 命中已有目标则合并,否则新建。这是"动态更新 stages/assistants"的基础:一个 stage 一个 key。
- **`pluginId + key` 才是全局身份** —— 不同插件允许产出相同 `key`;pipeline 要先生成
  `CardIdentity` / `CardId`,sink 的所有索引都使用它。不要只用裸 `key` 或裸 `cardKey`,
  否则两个插件会互相覆盖 anchor。
- pipeline 在把 `LiveCanvasMessage` 转成 sink 输入时会生成 `ResolvedCanvasMutation`,
  显式携带 sink 必需上下文: `pluginId`、`revision`、`sourceRuntimeSessionId`、
  `title`、`measure` / `placement` 与 ownership。不要让 sink 从闭包或全局状态里猜这些字段。
- **`revision` 单调,来源明确为 `snapshotEvent.sequence`** —— 对单源 key,sink 丢弃
  `revision <= 当前 key.revision` 的写入。对聚合 key,必须走下一条的 per-source map。
  不要让插件自造 revision;直接透传 snapshot event 的 `sequence`(runtimeChat 已用它做乱序过滤),
  否则抗乱序无可靠依据。
- **聚合 key 不能用单一 revision** —— `sequence` 是**按 session** 递增的。若一个 key 聚合多 session(如 workflow 主卡 `wf:{threadId}` 覆盖多个 stage/session),用全卡单一 revision 会把"低 sequence 但属于另一 session、仍然有效"的更新误丢。两种解法,择一并写清:
  - **(首选)拆细 key**:一个 session/stage 一个 key(`stage:{threadStageId}`),每 key 对应单一 session → 单一 revision 成立;主卡由 render 层聚合展示,不在 sink 层聚合。
  - **(备选)per-source revision map**:聚合 key 的状态里维护 `Map<sessioRuntimeSessionId, sequence>`,sink 按**来源**而非按整卡比较、丢弃,payload reducer 只更新该来源对应的分片。
- **`patch` 必须携带 reducer,不做隐式 merge** —— `payload` 是整体 JSON,`stages` / `assistants` 是数组。若 patch 用 shallow/deep merge,数组会出现"旧项残留"或"整段误替换"。因此 `patch` 定义为 `merge: (prev) => next`,由插件显式决定数组如何按 id 增删改(如 stages 按 `threadStageId`、assistants 按 `assistantId` 对齐)。sink 只负责调用 reducer + 比较 revision,不猜合并语义。
- **missing patch 不隐式建卡** —— 若 `patch` 命中不到现有 payload / anchor,sink 丢弃并
  debug warn。插件必须对首个实体或 `prev === null` 返回 `upsert`,避免 sink 用不完整上下文
  猜尺寸、placement、ownership。

#### Resolved mutation

```ts
export type ResolvedCanvasMutation<T = unknown> = CanvasMutation<T> & {
  identity: CardIdentity;
  cardId: CardId;
  pluginId: string;
  revision: number;
  sourceRuntimeSessionId: string | null;
  title: string;
  target: "overlay" | "doc";
  ownership: "live-anchor" | "live-owned" | "user-pinned";
  placementOwnership?: "auto" | "user-positioned";
  placement?: Placement;
  measuredSize?: { width: number; height: number };
};
```

### 4.1 卡片所有权三态(约束自动建/删块)

`upsert` / `remove` 会自动建 anchor 块或清理 live 目标,若无边界会塞满或误删用户画布。
因此每张卡片区分三态,sink 据此裁剪 mutation:

| 所有权 | 来源 | live `upsert` | live `remove` | 用户可编辑/移动 |
|--------|------|---------------|---------------|-----------------|
| **live-anchor overlay** | 低频 Yjs anchor + 高频 overlay payload | 建/复用 anchor,更新 overlay | 清 overlay + 删 anchor(未钉住时) | 可选中/移动;移动可记为 `placementOwnership=user-positioned`,显式 pin 才固化 |
| **live-owned persisted** | live 固化到 `payloadJson` 的块 | patch payload | **允许删**(仅未钉住时) | 是 |
| **user-pinned block** | 用户"钉住"/手动整理过 | **只 patch 内容(运行中 overlay,终态 doc 固化),不移动、不删** | **禁止删** | 是 |

规则:

- 首版默认走 **live-anchor overlay**:只低频创建/移动/删除 anchor,高频 payload 不进 Yjs。
- `placementOwnership` 是**位置所有权**,不是第四种 `ownership`。`user-positioned` 只表示
  sink 不再自动移动该卡;它仍可能是 `live-anchor`、仍可在过期/删除规则下被回收,除非用户显式 pin。
- `remove` **只能作用于未钉住的 live-anchor / live-owned 内容**;user-pinned 一旦形成,
  live 只能更新其 payload,不能删、不能改位置。
- 用户显式 pin / 保存为 revision 时,所有权升级为 `user-pinned`,退出 live 的自动删除域。
  升级时必须把当前 overlay payload 同步固化到 `payloadJson` + `revision`;否则刷新后
  pinned 卡只剩 anchor 空壳。
- **不要仅凭 `blockUpdated` 推断用户拖动。** sink 自己创建/移动 anchor 也会触发
  `doc.slots.blockUpdated`,需要 origin guard / suppress token 标记"本次写入来自 live sink",
  否则自动建卡会被误升级为 pinned。若要支持"用户一拖即 pin",必须只响应非 sink-origin
  的拖动/resize;更保守的首版可以把拖动记录为 `placementOwnership=user-positioned`
  (不再自动移动),
  但仍要求用户显式 pin 才固化 payload。
- `user-pinned` 后的内容更新必须明确走 **doc 模式** 或显式终态固化:若继续只写 overlay,
  刷新后 live patch 会丢;若写 `payloadJson`,会低频触发 autosave。建议策略是:pinned 卡
  运行中仍可 overlay 预览,在 turn 终态或用户保存时 diff 后写 `payloadJson`。

### 4.2 anchor 生命周期与准入策略

anchor 低频写入 Yjs 后会进入 canvas draft。由于 overlay payload 只在内存里,必须明确
anchor 的重载、清理和自动建卡边界:

- **load 清理**:加载 canvas snapshot 后,所有 `live-card` 若 `ownership === "live-anchor"`
  且 `payloadJson` 为空,视为临时 anchor。若当前 live pipeline 能立即重建 overlay,保留;
  若对应 plugin/key/source 已不可见或超出准入策略,在初始化阶段清理 anchor。
- **pin 固化**:用户显式 pin 或保存为 revision 时,将当前 overlay payload
  写入 `payloadJson`、写入 `revision`,并把 ownership 升级为 `user-pinned`。从此 live
  只能 patch 内容,不能删除或移动。
- **自动建卡准入**:sink 不应为所有 live message 自动建 anchor。默认只允许以下来源建卡:
  1. 当前打开/关注的 thread 或当前 canvas 已存在同 thread 卡片;
  2. 用户开启了对应 plugin 的 "show live cards";
  3. 未超过 plugin/key 的数量上限。
  未通过准入的消息只更新内部缓存或丢弃,不污染用户画布。
- **过期回收**:live-anchor 可带 `expiresAt` / `runId` / `sourceRuntimeSessionId` 元数据。
  运行结束且未 pinned 的 anchor 可在下一次 canvas 初始化或 pipeline idle 时回收。
- **load 清理会写 Yjs**,因此只能作为低频维护动作执行。初始化清理前应先完成一次
  admission / visibility 判断;若只是 overlay 暂未恢复,不要立刻删 anchor,避免打开画布时
  因竞态制造额外 autosave 或误删。

---

## 5. Registry 与运行管线

### 5.1 Registry(热插拔的载体)

```ts
// src/lib/canvas/live/registry.ts
export class LiveCanvasRegistry {
  private plugins = new Map<string, LiveCanvasPlugin>();
  private pluginInstances = new Map<string, symbol>();
  private listeners = new Set<() => void>();
  revision = 0;

  // host 构造注入:LiveCanvasHost 由 BlockSuiteCanvasHost 在挂载期创建并传入
  // (§10 接入点),registry 不自行持有 canvas doc/editor。
  constructor(private readonly host: LiveCanvasHost) {}

  register(plugin: LiveCanvasPlugin): Disposable {
    const id = plugin.id;
    const previous = this.plugins.get(plugin.id);
    const previousToken = this.pluginInstances.get(id);
    const instanceToken = Symbol(plugin.id);
    let previousUnmounted = false;
    try {
      previous?.onUnmount?.();
      previousUnmounted = Boolean(previous);
      this.plugins.set(id, plugin);
      this.pluginInstances.set(id, instanceToken);
      plugin.onMount?.(this.host);
      this.emit();
    } catch (error) {
      if (previous) {
        if (previousUnmounted) {
          try {
            previous.onMount?.(this.host);
            this.plugins.set(id, previous);
            if (previousToken) this.pluginInstances.set(id, previousToken);
          } catch {
            this.plugins.delete(id);
            this.pluginInstances.delete(id);
          }
        }
      } else {
        this.plugins.delete(id);
        this.pluginInstances.delete(id);
      }
      this.emit();
      throw error;
    }
    return { dispose: () => this.unregister(id, instanceToken) };
  }

  unregister(id: string, instanceToken?: symbol): void {
    if (instanceToken && this.pluginInstances.get(id) !== instanceToken) {
      return;
    }
    this.plugins.get(id)?.onUnmount?.();
    this.plugins.delete(id);
    this.pluginInstances.delete(id);
    this.emit();
  }

  get(id: string) { return this.plugins.get(id) ?? null; }
  list() { return [...this.plugins.values()]; }
  subscribe(fn: () => void) { this.listeners.add(fn); return () => this.listeners.delete(fn); }
  private emit() {
    this.revision += 1;
    this.listeners.forEach((fn) => fn());
  }
}
```

`register` 返回 `Disposable` → 这是热插拔的关键:调用方(内置插件的 `useEffect`、
未来的外部插件加载器)拿到句柄即可注销。
同 id 注册视为 replace:先 `onUnmount` 旧插件,再 mount 新插件。`Disposable` 必须带实例 token,
只允许卸载自己注册的那一版;旧 disposable 不能把后注册的新插件删掉。`revision` 每次
register/unregister 递增,供 `live-card` 的 `rerenderToken` 使用。
replace 必须保持 lifecycle 与 registry 状态一致:`onUnmount` / `onMount` 抛错时不能留下
Map、instance token、revision 互相矛盾的半状态。实现可用 try/catch 回滚到 previous
plugin,或采用先构造新实例成功后再提交 Map 的事务式顺序。

### 5.2 管线(runtime 统一编排,插件不接触落地目标)

```
agent-runtime-turn-snapshot(首版主源,见 §1.4)
  │
  ├─ registry.list().flatMap(p => p.ingest(ctx))     // 所有插件认领并解析
  │
  ├─ 生成 CardIdentity/CardId,按 identity + source revision 去重
  │   // 单源 identity 用 sequence;聚合 identity 用 per-source map
  │
  ├─ registry.get(pluginId).project(msg, prevPayload) // 收集 CanvasMutation[]
  │
  ├─ 160ms 批处理                                      // 复用现有节流窗口
  │
  └─ sink(唯一落地的地方,两种模式,见 §5.3):
      维护 CardId → blockId(anchor)索引
      upsert:通过准入且无 anchor → 建 anchor 块(低频写 Yjs);有 → 写 overlay[blockId]
      remove:清 overlay + 删 anchor(仅未钉住的 live-anchor/live-owned,见 §4.1/§4.2)
      写完更新 per-source revision map(见 §4 revision 条)
```

sink 是**唯一**落地 mutation 的地方,由此集中保证:写入节流、key 幂等、revision 单调。
但"落到哪里"不是一个而是**两种模式**,这是 review 后必须明确的关键分歧。
`CardId → blockId` 索引不能只在运行中累积:canvas snapshot 加载 / doc attach 后必须扫描
现有 `sessio:live-card` blocks,用 `pluginId + cardKey` 重建索引,再执行 load cleanup /
admission。否则重载后的首个 live `upsert` 会因为索引为空重复创建 anchor。若扫描到同一
`CardId` 的多个 anchor,必须确定性收敛:优先保留 `user-pinned`,否则保留最高 `revision`,
再否则保留最早创建/最小 block id;其余标为 orphan,按 §4.2 的低频 cleanup 规则回收。

### 5.3 落地目标:overlay 模式 vs doc 模式(默认 overlay)

上一份文档已核对出一条硬约束:`doc.slots.blockUpdated` **无条件**触发
`scheduleAutosave`(BlockSuiteCanvasHost.tsx:1270-1276)。因此:

> ⚠️ **"非 undo 事务" ≠ "非持久化 / 非 autosave"。**
> 只要写进 Yjs doc 并发出 `blockUpdated`,就会 churn canvas draft autosave,
> 与 live 高频更新叠加会造成持续落盘。

据此,sink 提供两种模式,**首版默认 overlay**:

| 模式 | 落点 | undo | autosave | 适用 |
|------|------|------|----------|------|
| **overlay(默认)** | 块外 React store `Map<blockId, {payload, revision}>`,承载块按自身 `blockId` 查并订阅(§3.0) | 不进 | **不触发** | 所有高频 live 运行态 |
| **doc(仅终态)** | `doc.updateBlock` / `insertEdgelessBlock` | 需绕过 | 会触发 | 用户"钉住"或权威终态,需持久化进修订 |

- **overlay 模式**:sink 只更新一个 React store,承载块渲染时**合并** `model.payloadJson`
  (持久态)与 overlay(瞬时态),overlay 优先。不写 Yjs → 不触发 autosave、不进 undo。
  这是 live 运行态的默认路径。
- **doc 模式**:仅在权威终态或用户显式固化时使用。启用前**必须先验证**存在一种
  BlockSuite 写法能同时"不进 undo 栈**且**不触发 `blockUpdated` → autosave";
  若无法两者兼得,doc 模式只在低频终态使用,接受其 autosave 代价。

> 这条把上一份文档"高频运行态放 React overlay,只有终态落 props"的结论,
> 提升为插件框架 sink 的一等模式,消除了此前 reconciler「只说 transient」的二义。

---

## 6. "动态更新 stages / assistants" 如何落到本架构

以 workflow 插件为例,展示 stages/assistants 的增量更新如何自然表达:

- **粒度选择**:首版推荐 **stage/session 细粒度 key**(`stage:{threadStageId}` 或
  `session:{sessioRuntimeSessionId}`),避免 workflow 主卡 `wf:{threadId}` 聚合多 session 后
  需要 per-source revision map。首版只要求 stage/session 卡各自更新;若要 workflow 总览,
  要么先补 §9.1 的 host payload selector 再由 render 层聚合多个 stage payload,
  要么等 per-source revision map 做完后再引入 `wf:{threadId}` 主卡。
- **ingest**:从 `snapshotEvent.session.turns` 中检测 `sessionUpdate` block 与
  **终态 turn status**(completed / failed / cancelled),结合 `mapping` 把 session
  关联到 thread,拉取/合并 `ThreadWorkSnapshot`(api.ts:2274),产出
  `LiveCanvasMessage<WorkflowPayload>`。**不要**去扩 `agent-runtime-event`(§1.4)。
  终态拉 snapshot 要**去重**:只在某 turn status 从非终态**跨变**为终态的那一次触发
  `getThreadWorkSnapshot`,避免每份含已完成 turn 的 snapshot 都重复拉。
  `WorkflowPayload` 直接复用 `stages: ThreadWorkSnapshotStage[]`、
  `assistants: StageAssistantInfo[]`(api.ts:2239, api.ts:433)。
- **project**:比对 `prev` 与新 payload —
  - stage 新增或 `prev === null` → `{ op: "upsert", key: "stage:...", payload }`
  - 已有 stage 内容变化 → `{ op: "patch", key: "stage:...", merge: prev => mergeStage(prev, stage) }`
  - stage 删除 → 若用了 stage 子卡则 `{ op: "remove", key: "stage:..." }`
  - assistants 变化 → 合入同一 reducer,由插件按 `assistantId` 决定增删改。
- **render**:`WorkflowLiveCard` 组件读取 `payload.stages` / `payload.assistants` 渲染,
  与现有 `WorkflowCardHost`(blocks/workflow-card/host.tsx)风格一致,可直接迁移复用。

因为 stage/session key 稳定 + 单源 `revision` 单调,同一 stage 的增删改收敛为对同一张卡的
幂等 patch,不会重复建卡,也不会因增量/快照并发而错乱。若未来引入 workflow 聚合主卡,
必须启用 §4 的 per-source revision map。

---

## 7. 与现有代码的衔接点

| 现有件 | 衔接方式 |
|--------|----------|
| **prop wiring 链路(P0 前置)** | `ChatPage → ChatCanvasView → BlockSuiteCanvasHost` 透传 `liveState` + `runtimeSessionAliases`(见 §1.5);管线在 canvas 侧运行的前提 |
| `acpRenderItems.ts` 的 `if (block.kind===...)` | **不在本议题内**(§1.1):transcript 渲染是独立线,不纳入 canvas live 衔接 |
| `workflow-card`(model/component/host) | 作为 **B 档**样板保留;或迁为 A 档 `live-card` 插件 |
| `portalBridge.ts` | 新增 `liveRegistry` 与 `liveHost` 字段,把 registry 透给 lit 块 |
| `useRuntimeEventSubscription.ts` | 事件进管线的入口;首版以 `agent-runtime-turn-snapshot` 为源(见 §1.4),复用其 160ms 节流窗口 |
| 关联层 `SessionThreadStageMap`(见 §7.1) | 作为 `IngestContext.mapping` |
| `placeCanvasNodes` / `insertEdgelessBlock`(BlockSuiteCanvasHost.tsx) | sink 的 doc 模式复用做布局与建块 |

### 7.1 关联层规格 `SessionThreadStageMap`

事件/快照只带 `sessioRuntimeSessionId`(+ `turnId`),而卡片按 `threadId` / `threadStageId`
归属。二者之间需要一个**双向索引**,不能只靠裸 `sessionId`——`sessionId` 会在
不同 agent / 子会话间碰撞,且 `getThreadWorkSnapshot(agent, sessionId)` 需要 child
**agent + session**,而非仅 `threadId`。

```ts
export interface SessionThreadStageMap {
  // 正向:运行时会话 → 归属实体(用于把 live 事件路由到卡片)
  bySessioRuntimeId: Map<string, {
    agent: Agent;
    childSessionId: string;         // 拉 snapshot 用
    sessioRuntimeSessionId: string;
    threadId: string | null;
    stageId: string | null;
    assistantId: string | null;
  }>;
  // 反向:thread → 该 thread 下所有卡片(fan-out 用)
  cardIdsByThread: Map<string, Set<CardId>>;
}
```

两个来源合成:

- `runtimeSessionAliases` 是 `{agent}:{sessionId} → sessioRuntimeSessionId`
  (见 ChatPage.tsx:398、useRuntimeEventSubscription.ts:34),需**反转**成
  `sessioRuntimeSessionId → {agent, childSessionId}`。
- `ThreadWorkSnapshotStage.sessionRefs[]`(api.ts:2250、2253)提供
  `{agent, sessionId} → {threadId, stageId}` 的绑定,补齐归属字段。

> 未命中(session 不属于任何已知 thread/stage)即丢弃,避免无关刷新。

---

## 8. 热插拔的诚实边界:A 档 / B 档

| 档位 | 机制 | 热插拔 | 能力 | 适用 |
|------|------|--------|------|------|
| **A 档** | `live-card` 承载块 + payload | ✅ 完全 | 自绘卡片,无原生富文本/子块 | 绝大多数 live message:状态卡、进度、指标、workflow/stage 摘要 |
| **B 档** | 插件自带独立 flavour(如 workflow-card) | ❌(构建期注册) | 原生编辑、嵌套、性能好 | 需要富交互/深度编辑的少数块 |

registry 契约**可以同时接受两档**,但**首版不要同时实现两档 sink**:
B 档受构建期 schema 约束,不是真热插拔(§1.2),把它接进同一 sink 会让首版背上
"原生块建删 + overlay 合并"两套落地逻辑。因此:

- **首版二选一**:要么只做 A 档 `live-card` overlay,要么直接走
  [workflow 文档](./canvas-workflow-live-update-plan.md)的现有 `workflow-card` + overlay
  最小闭环。**不做 B 档 sink。**
- B 档 `render.nativeFlavour` 指向真正的 BlockSuite 组件,仅作为**后续阶段**的扩展点保留;
  届时 sink 对 B 档走 `insertEdgelessBlock(nativeFlavour, ...)`。
- 两档共享 `ingest`/`project`/`key`/`revision` 契约,差异只在渲染落点。

> 代价明示:A 档卡片拿不到 BlockSuite 原生的富文本编辑与嵌套子块能力。
> 但换来的是"新增一种 live message 类型 = 写一个纯运行时插件 + 注册",不碰 schema、不改主流程。

---

## 9. 目录结构建议

```
src/lib/canvas/live/
  types.ts            # 插件契约、message、mutation
  registry.ts         # LiveCanvasRegistry
  pipeline.ts         # ingest → project → resolve mutation → 节流 → sink
  sink.ts             # 唯一落地 mutation 处;overlay / doc 两模式(见 §5.3);CardId↔目标索引
  overlayStore.ts     # 块外 React store:Map<blockId, {payload, revision}> + subscribe(blockId)
  host.ts             # LiveCanvasHost(暴露给插件的受限 API)
  plugins/
    workflow/         # workflow / stages / assistants 插件(A 档,首个插件)
    # 注:tool-activity / file-edits 属于 transcript 渲染议题(见 §1.1),
    # 不在本 canvas 框架首版范围内,勿在此处堆叠。

src/lib/blocksuite/blocks/
  live-card/          # 通用承载块(唯一新增的动态 schema)
    model.ts view.ts component.ts host.tsx index.ts
```

---

### 9.1 `LiveCanvasHost` 最小 API

准入、pin、跨组件渲染都不应从全局状态里猜。`BlockSuiteCanvasHost` 创建 pipeline 时,
应注入一个受限 host API:

```ts
export interface LiveCanvasHost {
  getActiveThreadId(): string | null;
  getCanvasThreadId(): string | null;
  isPluginEnabled(pluginId: string): boolean;
  getPluginCardLimit(pluginId: string): number;
  canAutoCreateCard(message: LiveCanvasMessage): boolean;

  getPayload<T = unknown>(identity: CardIdentity): T | null;
  subscribePayload(identity: CardIdentity, fn: () => void): () => void;

  markSinkWrite<T>(fn: () => T): T; // 给 blockUpdated/selection 监听区分 live sink 写入
  pinCard(blockId: string): void;   // overlay → payloadJson + ownership=user-pinned
}
```

首版如果不做跨卡聚合渲染,可以先不暴露 `getPayload/subscribePayload`;但只要 render
想从多个 stage 卡合成总览,就必须通过这类 host selector 读取 sibling payload,不能让
React 组件直接访问 sink 内部 Map。

### 9.2 `portalBridge` owner/scope

现有 `portalBridge` 是模块级 singleton,`BlockSuiteCanvasHost` 卸载时会
`setBlockSuitePortalBridge(null)`。加入 `liveRegistry/liveOverlay/liveHost` 后必须给 bridge
加 owner/scope 规则:

- bridge state 带 `ownerId`(每个 `BlockSuiteCanvasHost` mount 生成一次)。
- bridge state 带全局 `revision`,每次 `setBlockSuitePortalBridge` 递增,并提供
  `subscribeBlockSuitePortalBridge(fn)` / `getBlockSuitePortalBridgeRevision()`。这样已挂载的
  `live-card` 即使最初渲染时 bridge 为空,也能在 bridge ready 后 `requestUpdate()` 并重订阅。
- cleanup 只允许清理同 owner 的 bridge;旧 host 卸载不能把新 host 的 bridge 置空。
- `live-card` 订阅 overlay 时记录 store identity + ownerId;任一变化都释放旧订阅并重订阅。
- 若未来支持同屏多个 canvas,模块级 singleton 需要升级为按 editor/root element 取 bridge;
  在那之前文档和实现都应声明"同时只支持一个 active BlockSuite portal bridge"。

---

## 10. 落地顺序(风险从低到高)

> 前提:本框架是**较后阶段**的工作。若目标只是让 workflow 卡片尽快 live 更新,
> 应先走 [canvas-workflow-live-update-plan.md](./canvas-workflow-live-update-plan.md)
> 的 P0–P3(改现有 `workflow-card` + overlay),**不要**为单个卡片先建通用框架。
> 本节的 §1 起步 = 那份文档的 **P4**。

**接入点(定稿,消除此前二义):**
`useRuntimeEventSubscription` 只负责维护 **app-level `liveState`**(现状即如此),
**不持有 canvas doc/editor**。pipeline **运行在 `BlockSuiteCanvasHost` 内**:canvas 通过
props 拿到 `liveState` + `runtimeSessionAliases`(§1.5 的 wiring),在 host 内的 effect 里
把 snapshot 喂进 pipeline。这样 app-level hook 不触碰 canvas 生命周期。

1. **prop wiring(P0)**:`ChatPage → ChatCanvasView → BlockSuiteCanvasHost` 透传
   `liveState` + `runtimeSessionAliases`(§1.5)。与具体框架无关,最先落地。
2. **通用承载块 `live-card`(作 anchor)**:新增 schema + 注册进 `specs.ts`,先渲染 fallback。
   按 §3.0,首版 overlay **仍依赖** live-card 提供 blockId / 布局 / 缩放挂载点,
   只是 live payload 不进 props、走 overlay store。同步补齐 props defaults / nullable
   来源字段 / fallback migration(§3.1)。可独立验证建块/更新/删除。
3. **契约 + registry + pipeline + sink(overlay 模式)**:纯 TS,配单测
   (key 幂等、revision 抗乱序、mutation → overlay store)。sink 默认 overlay(§5.3),
   payload 写 `Map<blockId, payload>`,overlay store 按 blockId 通知 `live-card.requestUpdate()`,
   承载块渲染时复用 `getRenderingRect` 缩放(§3.0)。同时实现 `pluginId + key` 索引、
   owner-scoped portal bridge、sink-origin guard。
4. **管线接入**:在 `BlockSuiteCanvasHost` 内消费 props 传入的 `liveState`,喂进 pipeline,复用节流。
5. **第一个插件(workflow / stages / assistants)**:验证端到端动态更新;沿用 §6。首版只做
   stage/session 细粒度卡;若要 workflow 总览聚合,先补 host payload selector。
6. **doc 模式(仅终态)**:先验证"不进 undo 且不触发 autosave"的写法是否存在;
   据结论决定终态是否落 Yjs(§5.3)。
7. **(可选)外部插件加载器**:registry 已支持 `register/unregister`,后续可接入运行时装载。

> **不在本议题内**:`acpRenderItems` 的 transcript 渲染插件化(§1.1)是**另一条独立线**,
> 目标是聊天流而非 canvas block,不共用本框架的 sink / overlay / live-card。
> 不要把它挂进 canvas 落地顺序,否则 canvas 首版会背上 transcript 重构的包袱。

---

## 11. 待定问题

- **持久化**:live-card anchor 本身会低频写入 canvas draft;payload 默认不持久化。
  用户"钉住"后才把 payload 落进 `payloadJson` / revision(与上一份文档的 draft/revision 机制对齐)。
- **多插件认领同一事件**:是否允许?建议允许(fan-out),由 `key` 命名空间隔离(`pluginId` 前缀)。
- **payload schema 校验**:是否给 `payloadJson` 加运行时校验(zod)以防插件版本漂移。
- **B 档热重载**:开发期 `import.meta.hot` 已 reload;生产期 B 档仍受构建期约束,文档需向用户说明。
