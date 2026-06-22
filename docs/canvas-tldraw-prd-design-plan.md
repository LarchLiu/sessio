# Canvas Tldraw PRD / Technical Design Plan

## 摘要

在现有 `ChatPage` 的 `chat | file` 视图结构上新增 `canvas` 视图，使用 `tldraw`
提供 session 内的无限画布工作台。该画布用于把文档、图片、视频占位、workflow 和自由批注
放到同一空间中展示、组织和对话，同时继续复用 Sessio 已有的 composer、runtime session、
attachment 和 session 数据模型。

本方案不重做聊天系统，也不把 canvas 设计成独立产品。v1 的定位是：

- 当前 `ChatPage` 打开的 source session id 对应的一张单用户工作画布。
- 画布内容主要来源于 workspace 文件、当前会话编辑文件、本地图片/视频、用户创建或镜像的 workflow 和自由 note。
- 对话仍进入当前 runtime session 的主消息流。
- 画布为对话提供空间化上下文和截图能力。
- 历史 revision 以本地文件形式保存，SQLite 只保留索引和元数据。

## 背景与目标

当前 `ChatPage` 在 `file view` 下已经具备“展示 + 对话”骨架：主区域展示单文件视图，
底部 `SharedChatComposer` 常驻，因此用户可以一边看文件一边发问。

已确认的现状：

- `ChatPage` / `AcpTranscriptPanel` 是 session-first 页面，核心输入是 `SessionInfo` 而非 `ThreadInfo`。
- `chatView === "file"` 时主区域渲染 `ChatFilesView`。
- `SharedChatComposer` 在 detail chat 页面里常驻。
- 当前 file view 一次只支持单文件选择与查看，不支持多元素并置与空间组织。
- `ChatView` 不只影响 `ChatPage`，还影响 `navigation.ts`、`App.tsx`、`AppHeader.tsx` 和主视图切换入口。

相关位置：

- `src/pages/ChatPage.tsx:150`
- `src/pages/ChatPage.tsx:535`
- `src/pages/ChatPage.tsx:1039`
- `src/pages/ChatPage.tsx:1553`
- `src/pages/ChatPage.tsx:1657`
- `src/components/ChatFilesView.tsx:22`
- `src/navigation.ts:3`
- `src/App.tsx:145`
- `src/components/AppHeader.tsx:143`

### 职责边界原则

为避免 `ChatPage` 和 `ThreadMultiSessionChatPage` 的 workflow 语义混淆，本文统一采用以下边界：

- `ChatPage` 的 canvas 是 session-scoped 的设计工作台，优先承载当前 session 的文件整理、workflow 设计、截图提问和局部执行触发。
- `ThreadMultiSessionChatPage` 可以在后续承载 thread 语境下的 workflow 设计，以及该页面自身 workflow 的执行细节展示。
- 两个页面可以复用 workflow 节点模型、shape 和执行桥接能力，但 owner、上下文注入和默认交互仍按各自页面语境区分。
- 本文当前主实现范围聚焦 `ChatPage`；凡涉及 `ThreadMultiSessionChatPage`，仅描述边界和联动方向，不展开其完整交互与存储方案。

本方案的目标不是替换聊天，而是把当前“单文件 file view”升级为“多元素自由画布 view”。

### 产品目标

- 支持在同一 session 内自由组织多种内容节点。
- 支持对单节点或多选区发起对话。
- 支持对当前视口或选区截图，并把截图作为图片附件加入对话。
- 支持把 project 中任意文件加入画布。
- 支持把当前会话编辑文件快速加入画布。
- 支持在当前 session 内设计或镜像轻量 workflow。
- 支持让 workflow 节点可选接入 planner / astra 执行链。
- 支持在画布内创建 note / group 组织内容。
- 支持 reload / 切视图后恢复最近草稿状态。

### 非目标

- 不做多人实时协作。
- 不做完整视频理解管线。
- 不做通用流程图产品。
- 不把 canvas 独立成 project 级全局白板。
- 不改变现有 ACP transcript / agent runtime 主链路。
- 不处理 thread / stage / thread replay 的可视化节点。
- 不把 canvas workflow 节点和具体 `threadId` / `stageId` 绑定。
- 不在 `ChatPage` 里承载 planner / astra / thread workflow 的执行细节视图。
- 本方案当前落地范围优先 `ChatPage`；`ThreadMultiSessionChatPage` 的 workflow 设计与细节展示后续单独展开。

## 技术选型结论

Canvas v1 选用 `tldraw` 作为无限画布内核。

### 选择原因

- 需求核心是自由无限画布，而不是节点图编辑器。
- 需要 pan、zoom、drag、resize、marquee select、group 等白板交互。
- 需要承载文件卡片、图片卡片、视频占位卡片、workflow 卡片和 note 卡片等异构元素。
- 需要未来支持自定义 shapes、toolbar、context actions、export。

### 不选手写 / dnd-kit 的原因

当前仓库没有专用画布库，只有 `@dnd-kit/react`，更适合排序/拖放而不是无限画布。

相关现状：

- `package.json` 已安装 `@dnd-kit/react@^0.4.0`，无 `tldraw`、`@xyflow/react`、Konva、Fabric。
- 项目里的拖拽/缩放模式主要是局部 resize / 定位逻辑，不是通用画布引擎。

相关位置：

- `package.json:1`
- `src/layouts/AppLayout.tsx:155`
- `src/components/TerminalDock.tsx:350`

### 构建约束

当前 `vite.config.ts` 没有对大依赖做特殊 chunk 分割，因此 canvas 代码必须懒加载，
避免拖累 chat/file 首屏。

相关位置：

- `vite.config.ts:1`

## 用户画像与使用场景

### 目标用户

- 在 session 内同时处理多个文件、多个素材和多个截图的用户。
- 需要“边看边问、边整理边追问”的知识工作流用户。
- 需要把文件、图像、workflow、批注和输出放在同一视图理解关系的项目用户。

### 典型场景

1. 用户在 session 中打开 canvas，把需求文档、设计图、关键代码文件和最近编辑文件放在一张画布上。
2. 用户框选几份文档和一张图片，询问 agent “这些内容之间有哪些冲突？”
3. 用户把一个 thread workflow 镜像到画布，调整成当前 session 的执行草案，再对其中一段追问或触发 planner / astra。
4. 用户截取当前选区的拼贴视图作为图片附件，让 agent 基于视觉布局给建议。

## 信息架构

### 页面位置

在现有 detail chat 流程中新增第三种视图：

```ts
type ChatView = "chat" | "file" | "canvas"
```

canvas 不新开顶层页面，不脱离当前 session 语境，也不进入 `ThreadMultiSessionChatPage`。

### 接入范围

新增 `canvas` 视图不只是修改 `ChatPage` 分支，还要同步调整：

- `src/navigation.ts` 的 `ChatView`
- `src/App.tsx` 中的 `chatView` state、默认值和切换行为
- `src/components/AppHeader.tsx` 中的 `ChatViewToggle`
- `src/components/AppMain.tsx` 中传入 detail chat 页面时的 view 传播

这样才能保证 canvas 真正成为第一类视图，而不是只在 `ChatPage` 内部出现的隐藏分支。

### 页面结构

```text
ChatPage
  ├─ Header / view toggle
  ├─ Main view
  │   ├─ chat    -> transcript
  │   ├─ file    -> ChatFilesView
  │   └─ canvas  -> ChatCanvasView
  ├─ shared strips
  │   ├─ permissions
  │   ├─ minimal message strip
  │   └─ edited files bar
  └─ SharedChatComposer
```

### 核心原则

- composer 继续常驻，不在 canvas 内重复实现输入框。
- 画布是“上下文工作台”，主聊天流仍然是当前 runtime session 的事实源。
- 节点只承载展示和局部操作，不承载另一套独立消息系统。
- canvas 必须继续展示 pending permissions 和工作中状态，不能因为切到第三视图而丢失 session 状态感知。

## 用户流

### 用户流 1：打开和恢复 canvas

1. 用户在当前 session 内切换到 `Canvas` 视图。
2. 系统读取当前 session 对应的 canvas 文档元数据。
3. 若存在 draft snapshot 文件，则优先恢复最近 draft、缩放、相机和最近选中状态。
4. 若没有 draft 但存在最近已保存 revision，则恢复最后一次正式保存结果。
5. 若都不存在，则创建一个空白默认 canvas。

验收：

- 切到 canvas 时不影响当前 session/live 状态。
- 返回 chat/file 再切回 canvas 时状态保持。
- reload 后能恢复最近 draft 或最近已保存布局。

### 用户流 2：向画布添加内容

1. 用户从 “Add to canvas” 菜单选择来源。
2. 可选来源包括：
   - project 文件
   - 当前会话编辑文件
   - 本地图片文件
   - 本地视频文件
   - workflow
   - note
3. 系统创建对应节点并放置到默认位置或当前视口中心。

验收：

- 每种节点都能创建并持久化到 draft。
- 文件节点保留可追溯来源。
- workflow 节点保留结构化定义。

### 用户流 3：单节点提问

1. 用户单击一个节点。
2. composer 上方出现 scope 状态，例如 `1 selected`。
3. 用户输入问题并发送。
4. 系统把当前节点映射成 attachment 和 `canvasContext` 后发送给 agent。

验收：

- 节点相关问题进入当前 runtime session。
- agent 收到的上下文包含节点来源与必要摘要。

### 用户流 4：多选区提问

1. 用户框选多个节点。
2. 用户点击 `Ask selection` 或直接在 composer 中发送。
3. 系统生成选区上下文：
   - 选中节点列表
   - 每个节点的引用与摘要
   - 选区截图附件（可选/推荐）
4. 系统向 agent 发送一条普通输入。

验收：

- 多选问题可稳定带上所有选区元素。
- 截图和结构化摘要能同时进入上下文。

### 用户流 5：截图加入对话

1. 用户选择 `Attach selection snapshot`。
2. 系统从 tldraw 导出选区 PNG。
3. 系统把 PNG blob 写入本地缓存路径。
4. 图片附件自动加入 composer 预览区。
5. 用户补充文字并发送。

验收：

- 用户无需手动保存再选择文件。
- 图片能在 composer 中预览。
- agent 收到的是标准 image attachment。

### 用户流 6：画布批注

1. 用户对节点或选区创建 comment anchor。
2. 用户输入批注或问题。
3. 批注对应的聊天消息仍写入主 session。
4. UI 上将该 turn 与 anchor 建立关联，便于回看。

验收：

- 不引入第二套消息存储。
- 用户能从画布元素反查关联对话。

## 功能需求

### 核心交互

- pan
- zoom
- drag
- resize
- marquee select
- multi-select
- group / ungroup
- z-order
- fit to selection
- keyboard shortcuts

### 节点类型

#### file

- 展示文件名、路径、类型。
- 支持代码/文档预览。
- 支持打开原文件。
- 支持加入对话上下文。

#### image

- 展示缩略图。
- 支持放大查看。
- 支持直接作为 image attachment。

#### video

- v1 不实现完整本地视频播放器、poster 提取或首帧缓存。
- 仅支持“视频占位节点 + 文件来源信息 + 手动/后续截图”。
- 不直接作为 agent 附件原样发送。
- 发给 agent 时统一通过截图或摘要文件降级。

#### workflow

- 用于在当前 session 内设计轻量步骤流、检查清单或流程块。
- 可从空白创建，也可镜像 thread workflow 的结构作为起点。
- 展示标题、步骤列表、状态标签、备注和执行配置等结构化内容。
- 可选接入 planner / astra 执行链，但节点本身不绑定具体 `threadId` / `stageId`。
- 本方案在 `ChatPage` 中优先承载定义态和最小运行状态；`ThreadMultiSessionChatPage` 可在后续承载其自身 workflow 的设计与细节展示。
- 发给 agent 时生成 markdown 摘要文件。

#### note

- 轻量自由文本节点。
- 用于用户手写整理和拼贴说明。

#### group

- 用于组织多个节点。
- 可作为多选的语义容器。

### 对话作用域

composer 支持三种作用域：

- `canvas`：无选择时，针对整张画布。
- `selection`：单选或多选时，针对当前选区。
- `anchor`：从批注锚点发起时，针对对应节点或选区。

### 截图导出

支持三种导出模式：

- 当前选区
- 当前视口
- 整张画布

v1 主推：

- 当前选区
- 当前视口

## 已探查的现有能力与约束

### ChatPage 架构与视图切换

当前 `ChatPage` 已具备 file/chat 切换与常驻 composer，但 `isFilesView` 之外的分支目前都默认按 transcript 处理，
因此新增 canvas 必须把 `chat`、`file`、`canvas` 明确拆成三路，而不是继续沿用“非 file 就是 transcript”的二分逻辑。

相关位置：

- `src/pages/ChatPage.tsx:1039`
- `src/pages/ChatPage.tsx:1553`

### 底部 strips 现状

当前 permissions、minimal message strip、edited files bar 只在 file view 渲染，因此若 canvas 也需要保持 live session 可见性，
必须把这些 shared strips 提升为 file/canvas 共用，或重新定义一套统一的 “non-chat content views” 布局策略。

相关位置：

- `src/pages/ChatPage.tsx:1657`

### 附件数据模型

现有附件协议：

```ts
interface AgentAttachment {
  path: string
  mimeType: string | null
  kind: "image" | "file"
  previewDataUrl?: string | null
  displayName?: string | null
}
```

相关位置：

- `src/api.ts:1113`

含义：

- 当前 runtime 只原生支持 `image` 和 `file` 两类附件。
- canvas 不应扩展出 `video` 或 `workflow` 作为新的 attachment kind。
- 视频和 workflow 必须在发送前降级成现有可接受格式。

### Composer 附件能力

现有 `useComposerAttachments` 已具备：

- 添加路径附件
- 读取图片预览
- 粘贴图片/文本文件
- 去重与移除

相关位置：

- `src/components/ComposerAttachments.tsx:107`
- `src/components/ComposerAttachments.tsx:128`
- `src/components/ComposerAttachments.tsx:163`
- `src/components/ComposerAttachments.tsx:217`

现状限制：

- `addAttachments` 当前是 hook 内部能力，没有对外暴露。
- 其内部实现主要负责状态更新和图片预览，不是完整的 capability-aware 校验入口。
- 若 canvas 要“把截图自动塞进 composer”，应暴露正式的编程式追加 API，并在该 API 内继续校验：
  - 当前 agent 是否支持 image/file attachments
  - 文件大小是否超限
  - 需要时是否能读取预览

结论：

- 不建议只把内部 `addAttachments` 裸露出来。
- 更适合在 `useComposerAttachments` 或 `useChatComposer` 层提供安全的 `appendAttachments(drafts)`。

### 现有发送链路

`ChatPage` 在发送时会把 composer attachments 转成 `AgentAttachment[]` 并调用
`sendAgentInput`。

相关位置：

- `src/pages/ChatPage.tsx:1191`
- `src/pages/ChatPage.tsx:1372`
- `src/api.ts:2471`

### Runtime options 现状

`AgentInput.options` 当前可以透传任意 `RuntimeMetadata`，但仓库里没有现成的 `canvasContext`
消费逻辑。仅在前端塞入 `options.canvasContext` 并不会自动影响 agent 回答，必须补充
runtime / prompt 装配侧消费者。

相关位置：

- `src-tauri/src/agents/runtime/types.rs:144`
- `src-tauri/src/lib.rs:4383`
- `src-tauri/src/astra/mod.rs:2883`

### 图片预览与粘贴缓存能力

前端：

- `readLocalImageDataUrl(path)` 可把本地图片转成 data URL。
- `savePastedAttachment(req)` 可把 base64 写入本地缓存路径。

相关位置：

- `src/api.ts:2269`
- `src/api.ts:2273`

后端：

- `read_local_image_data_url` 读取绝对路径图片，大小上限 24MB。
- `save_pasted_attachment` 把 base64 写入粘贴缓存目录，大小上限 32MB。

相关位置：

- `src-tauri/src/lib.rs:2818`
- `src-tauri/src/lib.rs:2837`

结论：

- canvas 截图可以直接复用现有 “导出 blob -> base64 -> save_pasted_attachment -> attachment”
  链路。
- v1 不需要新增原生截图命令。

### 历史文件型上下文注入模式

当前项目已经有“先生成 markdown 文件，再作为 file attachment 注入”的模式，例如
cross-context。

相关位置：

- `src/pages/ChatPage.tsx:3850`
- `src-tauri/src/lib.rs:4024`

结论：

- 多选摘要适合沿用这条模式：先生成临时 `.md`，再当作 file attachment。
- workflow 节点摘要也适合沿用这条模式：先生成临时 `.md`，再当作 file attachment。

### 文件持久化模式

当前仓库已经存在多种“本地文件落盘 + SQLite 或运行时引用”的模式：

- cross-context 写到 `~/.sessio/projects/.cross-context`
- pasted attachments 写到 `~/.sessio/paste-cache`
- Astra 产物写到 workspace `.sessio/astra`

相关位置：

- `src-tauri/src/app_paths.rs:53`
- `src-tauri/src/app_paths.rs:57`
- `src-tauri/src/app_paths.rs:61`
- `src-tauri/src/astra/mod.rs:192`

结论：

- canvas snapshot / revision 存成本地文件是符合仓库现有设计风格的。
- SQLite 更适合只存索引、路径、hash、时间戳和关联关系。

## 数据模型

### 设计原则

- `tldraw` snapshot 文件是画布布局和几何状态的真相源。
- SQLite 只做索引、关联和附加元数据，不保存大块 snapshot JSON。
- session-scoped canvas 是 v1 主模型。
- revision 与 draft 都落本地文件；SQLite 保存可恢复入口和历史索引。
- 不把每个 shape 字段拆成细粒度 SQL 列。
- shape props 只保留渲染和交互所需的最小字段，不承载重业务语义。
- 节点与业务对象的关联、来源、摘要和附加业务元数据统一通过 sidecar 索引表维护。

### 逻辑模型

```ts
type CanvasNodeKind = "file" | "image" | "video" | "workflow" | "note" | "group"

interface CanvasDocument {
  id: string
  sessionId: string
  title: string
  currentSavedRevision: number | null
  draftSnapshotPath: string | null
  draftSnapshotHash: string | null
  draftUpdatedAt: number | null
  createdAt: number
  updatedAt: number
}

interface CanvasRevision {
  id: string
  canvasId: string
  revision: number
  snapshotPath: string
  snapshotHash: string
  snapshotSizeBytes: number
  createdAt: number
  source: "manual" | "migration"
}

interface CanvasShapeRef {
  id: string
  canvasId: string
  shapeId: string
  kind: CanvasNodeKind
  sourceType: "workspace_file" | "attachment_file" | "attachment_image" | "video_file" | "workflow_definition" | "note"
  sourceKey: string | null
  sourcePath: string | null
  metadataJson: string
  createdAt: number
  updatedAt: number
}

interface CanvasContextAnchor {
  id: string
  canvasId: string
  anchorShapeId: string | null
  selectionShapeIdsJson: string
  turnId: string
  summary: string | null
  createdAt: number
}
```

其中 workflow 节点的业务元数据建议落在 `metadataJson`，例如：

```ts
interface WorkflowNodeMetadata {
  title: string
  steps: Array<{
    id: string
    label: string
    status: "pending" | "running" | "done" | "blocked"
    notes?: string | null
  }>
  mirrorSource?: {
    kind: "thread_workflow"
    label?: string | null
  } | null
  execution?: {
    enabled: boolean
    driver: "planner" | "astra" | null
    lastRunId?: string | null
    lastStatus?: "idle" | "running" | "succeeded" | "failed" | null
  } | null
}
```

约束：

- 可记录“来源于某个 thread workflow 的镜像”这一语义。
- 不把具体 `threadId` / `stageId` 作为 shape 主身份或必填持久化字段。
- planner / astra 运行结果只保留最小执行状态与引用，不把 thread replay 数据整块写进 shape。

### 作用域说明

v1 的 canvas 绑定到当前打开的 source session id，而不是 thread，也不跟 composer 当前 target agent 绑定：

- `ChatPage` 当前是 session-first 页面。
- workflow 节点可以镜像 thread workflow 的结构，但 canvas owner 和节点主身份仍然是 session-local。
- 若 workflow 节点接入 planner / astra，只保留抽象执行配置和运行引用，不反向把具体 thread 信息写成强绑定。
- planner / astra / thread workflow 的 timeline、session lane、replay 细节不进入 `ChatPage` canvas，统一留给 `ThreadMultiSessionChatPage`。
- 同一 `ChatPage` 内允许切换 target agent 发送，但这不改变 canvas owner。
- `ThreadMultiSessionChatPage` 可在后续拥有自己的 workflow 设计与细节展示能力，但不影响当前 `ChatPage` 的 session-scoped canvas 方案。

## 发送上下文模型

### 前端发送对象

发送给 runtime 时新增：

```ts
interface CanvasContextOption {
  canvasId: string
  scope: "canvas" | "selection" | "anchor"
  shapeIds: string[]
  anchorId?: string | null
  snapshotAttachmentPath?: string | null
  refs: Array<{
    shapeId: string
    kind: CanvasNodeKind
    sourceType: string
    sourcePath?: string | null
    sourceKey?: string | null
    summary?: string | null
  }>
}
```

此对象进入：

- `AgentInput.options.canvasContext`

### Runtime 消费要求

仅把 `canvasContext` 塞进 `AgentInput.options` 不足以完成需求。v1 必须明确实现以下消费者：

1. runtime 侧读取 `options.canvasContext`
2. 在 prompt 装配前把它转成稳定的结构化上下文
3. 对超长 refs / 摘要进行截断和归一化
4. 在需要时补充：
   - 选区截图 attachment
   - 多选摘要 markdown attachment
   - workflow 摘要 markdown attachment

推荐做法：

- 新增 `buildCanvasPromptContext` 或等价 helper
- 在 `sendAgentInput` 所在的 runtime prompt 装配路径里消费它
- 最终让 agent 看到的是一段稳定 prompt block + 标准 attachments，而不是依赖 agent 原生认识 `canvasContext`

### 持久化策略

- `canvasContext` 默认只作为一次发送时的 runtime 选项使用。
- 不把完整 `canvasContext` 原样写进 session history。
- 需要回看关联时，依赖 `canvas_context_anchors` 和标准 turn / attachment 记录。

## 组件边界

### 页面层

#### `ChatPage`

职责：

- 视图切换
- session 级状态与 live 绑定
- composer 复用
- shared strips 复用

变更：

- 新增 `isCanvasView`
- 把 `chat`、`file`、`canvas` 明确拆成三路渲染
- 把 permissions / minimal strip / edited files bar 提升为 file/canvas 共用布局能力

#### `ChatCanvasView`

职责：

- canvas 页面容器
- toolbar
- empty state
- 选区状态展示
- 右侧属性面板与快捷动作

### tldraw 桥接层

#### `TldrawCanvasHost`

职责：

- 封装 tldraw editor/store
- 初始化 snapshot
- 订阅变更并触发 draft autosave
- 对 autosave 做 debounce / in-flight 合并 / 写失败回退
- 处理 selection/camera/export 事件

#### `canvas/shapes/*`

职责：

- 各类自定义 shape 的定义与渲染
- shape 元数据与 UI 控件

计划节点：

- `fileShape`
- `imageShape`
- `workflowShape`
- `noteShape`
- `groupShape`

### 业务桥接层

#### `useCanvasPersistence`

职责：

- 加载 canvas 文档元数据
- 读取 / 写入 draft snapshot 文件
- 手动保存时创建 revision 文件与索引
- 同步 shape refs
- 在 session truly deleted 时清理失效文件

#### `useCanvasContextBridge`

职责：

- 读取当前 selection
- 把 selection 转成：
  - `AgentAttachment[]`
  - `options.canvasContext`
  - workflow 摘要文件
  - screenshot 附件

#### `useCanvasScreenshot`

职责：

- 导出选区/视口 PNG
- blob -> base64
- 调用 `savePastedAttachment`
- 生成 composer 可接受的 attachment draft

#### `useCanvasWorkflowNode`

职责：

- 创建和编辑 workflow 节点的结构化定义
- 支持从 thread workflow 镜像一份 session-local workflow 定义
- 管理 planner / astra 执行配置与最小运行状态
- 生成可供提问的 markdown 摘要文件
- 提供与 `ThreadMultiSessionChatPage` 的联动入口

### UI 辅助层

#### `CanvasInspector`

职责：

- 展示节点元信息
- 展示来源与关联对象
- 展示最近相关 turn / anchor

#### `CanvasAssetPicker`

职责：

- 从 project files、edited files、本地图片/视频和 workflow 来源中选源创建节点

## 存储设计

### 文件布局

推荐在 app home 下新增 canvas 目录，例如：

```text
~/.sessio/projects/.canvas/
  <session-id>/
    draft.tldr.json
    revisions/
      000001.tldr.json
      000002.tldr.json
    exports/
      snapshot-<ts>.png
    context/
      selection-<ts>.md
      workflow-<ts>.md
```

原则：

- draft 始终是最近工作态
- revision 仅在手动保存时新增
- 历史 revision 也只存文件，SQLite 只记路径和索引
- 导出图片与上下文摘要文件可放在同级目录或既有缓存目录

### SQL 草案

```sql
create table canvases (
  id text primary key,
  session_id text not null,
  title text not null,
  current_saved_revision integer,
  draft_snapshot_path text,
  draft_snapshot_hash text,
  draft_updated_at integer,
  created_at integer not null,
  updated_at integer not null,
  unique (session_id)
);

create table canvas_revisions (
  id text primary key,
  canvas_id text not null,
  revision integer not null,
  snapshot_path text not null,
  snapshot_hash text not null,
  snapshot_size_bytes integer not null,
  source text not null,
  created_at integer not null,
  unique (canvas_id, revision),
  foreign key (canvas_id) references canvases(id) on delete cascade
);

create table canvas_shape_refs (
  id text primary key,
  canvas_id text not null,
  shape_id text not null,
  kind text not null,
  source_type text not null,
  source_key text,
  source_path text,
  metadata_json text not null,
  created_at integer not null,
  updated_at integer not null,
  unique (canvas_id, shape_id),
  foreign key (canvas_id) references canvases(id) on delete cascade
);

create table canvas_context_anchors (
  id text primary key,
  canvas_id text not null,
  anchor_shape_id text,
  selection_shape_ids_json text not null,
  turn_id text not null,
  summary text,
  created_at integer not null,
  foreign key (canvas_id) references canvases(id) on delete cascade
);
```

### 约束说明

- `unique (session_id)` 实现“一个 session 一个默认 canvas”
- `unique (canvas_id, revision)` 避免 revision 冲突
- 所有关联表使用 `on delete cascade`
- 不尝试对 `sessions(agent, session_id, scope)` 建正式复合外键，避免把 canvas 表强耦合到现有 session 去重/placeholder 机制
- 读写逻辑以当前 `ChatPage` 打开的 source `session.id` 为唯一 owner 来源
- 此处明确忽略 composer target agent；target agent 改变不迁移 canvas owner

### 存储策略

- `canvases`：session 级 canvas 文档头和 draft 入口。
- `canvas_revisions`：正式保存历史的索引，不保存 snapshot JSON 本体。
- `canvas_shape_refs`：shape 到 Sessio 业务对象的索引映射。
- `canvas_context_anchors`：画布元素与 session turn 的关联。

### 为什么不把 snapshot 放进 SQLite

- tldraw snapshot 体积大且更新频繁。
- SQLite 更适合存索引，不适合承载高频大文本写入。
- 文件存储更接近仓库里已有的 cross-context / paste-cache / astra artifacts 模式。
- 文件形式也更方便后续做 retention、hash 去重和离线调试。

### Autosave 与手动保存

v1 推荐策略：

- 编辑时对 `draft.tldr.json` 做 debounce 写入，建议 500-1000ms 窗口并合并连续变更
- 同一时刻只允许一个 draft 写入进行中；新变更与进行中的写入合并
- draft 写入失败时保留内存态，并在 UI 标记 unsaved / save failed
- draft 损坏或读取失败时回退到最近 saved revision
- 用户主动点击 `Save canvas` 时：
  - 复制或 promote 当前 draft 为正式 revision 文件
  - 新增 `canvas_revisions` 索引行
  - 更新 `canvases.current_saved_revision`

这意味着：

- 未显式保存的工作也能在 reload 后恢复
- 历史 revision 不会因为频繁编辑无限膨胀
- 正式版本边界由用户控制

### Orphan 清理策略

文件清理不依赖 SQLite cascade，v1 明确采用以下规则：

- `canvas_revisions` / `canvas_shape_refs` / `canvas_context_anchors` 由 SQLite `on delete cascade` 清理
- 本地 `draft/revisions/exports/context` 文件由 canvas 文件清理器删除
- 只有在 session truly deleted 时才删除 `.canvas/<session-id>/` 目录
- session 仅被标记 `available = 0` 或 `archived = 1` 时，不自动删除 canvas 文件
- 定期清理任务可移除已无 SQLite 记录且超过保留窗口的孤儿目录

### Retention / Compaction

为了避免 revision 和草稿文件无限增长，v1 就应明确：

- draft 始终只有 1 份当前文件
- revision 只在手动保存时创建
- 可设置保留最近 N 个 revision，或在后续版本引入归档清理
- screenshot / 临时 markdown 文件需要有清理策略

## 上下文注入设计

### 目标

当用户在 canvas 中提问时，agent 需要同时理解：

- 用户选中了什么
- 每个元素来自哪里
- 这些元素的文字/图像摘要是什么
- 是否存在视觉截图

### 注入策略

#### 文件节点

- 直接转成 `kind: "file"` attachment。
- 若文件过大或不适合直接传输，仅在 `canvasContext.refs` 中传路径与摘要。

#### 图片节点

- 直接转成 `kind: "image"` attachment。
- 若只有本地路径无预览，则复用 `readLocalImageDataUrl(path)`。

#### 视频节点

- 不直接发原视频。
- 优先转为：
  - 当前视频节点 poster/截图
  - 一份 markdown 摘要文件

#### workflow 节点

- 生成 markdown 摘要文件。
- 摘要包含标题、步骤、状态、备注和可见执行状态。
- 若节点来自 thread workflow 镜像，仅写入归一化结构，不注入具体 thread 标识。
- 不把 thread replay / astra timeline 细节直接注入当前对话；这些细节由 `ThreadMultiSessionChatPage` 承载。
- 摘要模板由后端统一生成，前端只传最小业务输入。

#### 多选区

- 生成一份结构化摘要 markdown。
- 推荐同时附加选区 PNG 截图。

### 发送载体

```ts
sendAgentInput(sessioRuntimeSessionId, {
  text,
  attachments,
  options: {
    canvasContext
  }
})
```

相关现有发送入口：

- `src/api.ts:2471`

### Prompt 装配建议

在 runtime 消费侧，将 `canvasContext` 规范化为如下 prompt block：

```text
[Canvas context]
Canvas scope: selection
Selected items:
1. file - docs/prd.md
2. image - mock.png
3. workflow - implementation plan
Use attached screenshot and summary files when answering.
```

这样：

- agent 不需要原生理解自定义 JSON
- prompt 内容可控、可截断
- 行为更容易测试

### 归一化决策

v1 直接拍板：

- workflow 摘要 markdown 由后端统一生成
- `canvasContext` prompt block 由后端统一归一化
- 前端负责采集 selection、shape refs、截图路径和最小输入数据

理由：

- 更贴近当前 cross-context 的文件注入模式
- 更容易统一截断策略、字数控制和测试
- 避免前后端各自拼 prompt 造成漂移

## 截图设计

### 设计原则

- v1 只做“画布内容导出”，不做 OS 级屏幕捕获。
- 尽量复用现有 attachment 缓存能力。
- 截图加入 composer 时必须经过支持能力校验。

### 实现路径

1. 从 tldraw 导出当前选区或视口图片。
2. 获取 PNG blob。
3. 前端将 blob 转 base64。
4. 调用 `savePastedAttachment({ fileName, mimeType, dataBase64 })`。
5. 获得本地缓存 `path`。
6. 调用正式的 `appendAttachments(drafts)` 能力把 canvas 截图加入 composer。

### 需要的前端改动

- 给 `useComposerAttachments` 暴露 capability-aware 的编程式追加 API。
- 或在 `useChatComposer` 层新增 `appendAttachments(drafts)`，内部复用统一校验逻辑。
- 不直接暴露未校验的内部 `addAttachments`。

## 交互与 UI 要点

### 画布顶部工具栏

- Add to canvas
- Ask selection
- Attach selection snapshot
- Save canvas
- Fit
- Zoom controls
- Group / Ungroup

### 右侧属性面板

- 节点类型
- 来源信息
- 文件路径 / 本地资源信息
- workflow 定义 / 执行状态
- 最近关联对话
- 快捷动作：
  - Open source
  - Add to prompt
  - Snapshot
  - Create anchor
  - Open execution details

### composer 作用域提示

建议在 composer 顶部显示小型 scope chip：

- `Canvas`
- `1 selected`
- `4 selected`
- `Anchor`

### shared strips

canvas 模式下仍应显示：

- pending permissions
- minimal working strip
- edited files bar

这部分不要继续限定在 file view。

### 空态

空画布时给出快速入口：

- Add project file
- Add edited files
- Add recent image
- Create workflow
- Create note

## 分阶段里程碑

### Phase 1: View Shell

目标：把 canvas 作为第三视图正式接入，并建立 session-scoped canvas 元数据与 draft 文件恢复能力。

执行内容：

- 扩展 `ChatView` 为 `chat | file | canvas`
- 更新 `navigation.ts`、`App.tsx`、`AppHeader.tsx`、`AppMain.tsx`
- `ChatPage` 新增 `isCanvasView`
- 把 `chat` / `file` / `canvas` 拆成三路渲染
- 新建 `ChatCanvasView`
- 懒加载 `tldraw`
- 新增 `canvases` 表
- 接入 draft snapshot 文件读写
- 落地 autosave debounce、写失败标记和 last revision 回退
- 调整 shared strips，让 file/canvas 都能看到 session 状态

验收：

- 能切换到 canvas
- 能看到空白画布
- reload 后能恢复 draft
- draft 写入失败时不会覆盖最近 saved revision，且 UI 有可见错误状态
- 不影响现有 chat/file
- canvas 下仍可见 pending permissions 和工作状态

### Phase 2: Core Nodes And Persistence

目标：支持最小节点集合与可用的文件化持久化。

执行内容：

- 实现 `file` / `image` / `workflow` / `note` / `group` 节点
- 完成 add-to-canvas 流程
- 完成拖拽、缩放、多选、分组
- 建立 `canvas_shape_refs`
- 实现手动 `Save canvas`
- 新增 `canvas_revisions` 索引表
- 历史 revision 以文件形式写入

验收：

- 用户可把文件、图片和 workflow 放到画布
- reload 后 draft 和节点位置保持
- 手动保存后能生成正式 revision
- 文件节点能回到原文件
- workflow 节点定义能稳定恢复

### Phase 3: Context Bridge

目标：把 canvas 真正接入对话，并补齐 runtime 消费链路。

执行内容：

- 实现 `useCanvasContextBridge`
- 暴露 composer 安全的编程式追加 attachment 能力
- 实现 screenshot -> attachment
- 单选 / 多选提问
- `canvasContext` 注入
- 在 runtime / prompt 装配侧消费 `canvasContext`
- workflow 节点 markdown 摘要注入

验收：

- 能对节点或选区提问
- 图片截图能自动加入 composer
- workflow 节点能以文件摘要进入上下文
- agent 的实际回答能反映 `canvasContext` 提供的信息

### Phase 4: Workflow / Anchor / Inspector

目标：补齐 workflow 执行桥、批注锚点和检查器能力，同时保持执行细节页边界清晰。

执行内容：

- 实现 `useCanvasWorkflowNode`
- 接入 workflow -> planner / astra 执行桥
- 实现 `canvas_context_anchors`
- 右侧 inspector
- anchor 与 turn 关联展示
- 增加与 `ThreadMultiSessionChatPage` 的联动入口

验收：

- workflow 节点可触发执行并展示最小运行状态
- `ChatPage` 与 `ThreadMultiSessionChatPage` 的 workflow 信息边界保持清晰
- 用户能创建并回看 anchor

### Deferred: Video

目标：在基础能力稳定后，再评估是否值得把视频从“占位 + 截图降级”升级为更完整的播放器节点。

进入条件：

- 基础 canvas 已稳定
- 用户确实需要视频内联浏览
- 已验证跨平台格式和 poster 策略

### Phase 5: Polish

目标：稳定性、性能和体验收尾。

执行内容：

- draft autosave 节流与容错
- revision retention / cleanup
- lazy load / bundle 控制
- 错误恢复
- 快捷键与交互微调
- 空态、文案、权限、边界条件完善

验收：

- 首屏无明显回归
- 100+ 节点画布的基本操作仍可接受
- 导出单张 4K 级 PNG 时无明显卡死或崩溃
- 错误路径明确
- 文件数量和缓存不会无限增长

## 测试与验证建议

至少覆盖以下回归场景：

- `chat -> file -> canvas -> chat` 切换不丢状态
- canvas 下 pending permissions 仍可处理
- draft 在 reload 后恢复
- 手动保存会新增 revision 文件与 SQLite 索引
- 删除 session / 清理 owner 后不会残留孤儿索引
- programmatic attachment 注入仍遵守 agent capability 限制
- `canvasContext` 在 runtime 消费后确实进入 prompt / attachments
- 100+ 节点下基本交互不出现明显失真
- draft 写入失败时能回退到最近 saved revision

## 风险与开放问题

### 风险

1. `tldraw` 体积较大，若不懒加载会拖慢主包。
2. 多选摘要或 workflow 摘要若过长，可能让上下文注入失控。
3. 画布截图若频繁导出大图，可能带来内存压力。
4. 若未来恢复完整视频节点，跨平台格式兼容和 poster 提取仍是独立风险。
5. 若违反 “shape 最小化 + sidecar 承载业务语义” 约束，会增加后续升级 tldraw 的迁移成本。
6. 若 draft 文件写入失败或损坏，需要确保 last revision 回退路径可靠。
7. 历史 revision 全部用文件存储后，清理和 orphan 检测需要专门机制。
8. workflow 节点若同时镜像结构又接执行链，需避免定义态与运行态漂移。

### 开放问题

1. 视频是否值得从“占位 + 截图降级”升级到更完整节点。
2. `ThreadMultiSessionChatPage` 的 workflow 设计能力应与 `ChatPage` 复用多少模型与组件。
3. workflow 节点与 planner / astra 执行结果之间的同步粒度要做到多细。

## 推荐实施顺序

建议按照以下顺序推进：

1. `Phase 1 View Shell`
2. `Phase 2 Core Nodes And Persistence`
3. `Phase 3 Context Bridge`
4. `Phase 4 Anchor / Inspector`
5. `Phase 5 Polish`

理由：

- 先把第三视图接入做完整，避免只改 `ChatPage` 内部造成半接入状态。
- 先验证 session-scoped draft/revision 文件化持久化，再逐步引入 workflow 定义与执行桥。
- 对话桥接是该功能的真正价值点，但必须在 runtime 消费链路明确后再落地。
- workflow 执行桥和 anchor 都可以在基础能力之上迭代，video 则继续 deferred。

## 结论

这个方案的本质不是“把 tldraw 塞进 Sessio”，而是：

- 复用现有 `ChatPage` 架构，但把它正式扩展为 `chat | file | canvas`
- 复用现有 composer 和 attachment 管线
- 用 `tldraw` 提供无限画布交互
- 用文件保存 draft 和历史 revision
- 用少量 SQLite sidecar 表把画布节点和 Sessio 的 session/file/workflow 体系连接起来

这样能在不推翻现有聊天系统的前提下，把单文件 file view 升级为面向多素材、多产物、
多轮追问的 session-scoped 空间化工作台。
