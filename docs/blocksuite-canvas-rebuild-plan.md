# Canvas BlockSuite Rebuild Plan

## 摘要

本文定义 Sessio canvas 从 `tldraw` 重构到 `BlockSuite` 的完整方案。

本次重构采用以下核心决策：

- 新画布直接按 `BlockSuite` 的数据模型与交互模型实现。
- 不考虑旧 `tldraw` 画布数据迁移。
- 不考虑旧 `shape` 命名、SQLite 字段、Tauri command 和 runtime metadata 兼容，直接全链路改为 `block` 语义。
- `BlockSuite` 负责新的白板与结构化内容承载层。
- 现有 raw markdown 编辑和 markdown 渲染链路继续保留。
- 现有 markdown 预览能力会以“block shell + lazy preview body”的形式嵌入到 `BlockSuite edgeless` 白板中。

这意味着本方案不是“把所有文档能力一次性全部换成 BlockSuite”，而是：

- 用 `BlockSuite EdgelessEditor` 取代 `tldraw` 无限画布。
- 用自定义 block 承载文件卡片、workflow 卡片和 markdown 预览卡片。
- 保留 `ChatFilesView` / `PlainEditorView` / `PlainMarkdownPreview` 作为 repo 文件查看与编辑主链路。
- 在合适阶段再引入 `BlockSuite PageEditor`，承接结构化文档、AI 生成文档和白板内文档内容。

本文是新的主方案，原 `docs/canvas-tldraw-prd-design-plan.md` 仅保留为历史方案参考。

## 背景

当前 canvas 实现位于：

- `src/components/TldrawCanvasHost.tsx`
- `src/canvasTypes.ts`

当前 markdown 文件查看与编辑主链路位于：

- `src/components/ChatFilesView.tsx`
- `src/components/PlainEditorView.tsx`
- `src/components/PlainMarkdownPreview.tsx`

现状特征：

- `tldraw` 已不仅是绘图层，还承载文件节点、workflow 节点、note 节点、上下文注入、anchor 和 revision 保存。
- markdown 文件预览已支持 GFM、KaTeX、Mermaid、DOT、Vega、Vega-Lite、本地图片解析、相对路径与安全清洗。
- 当前 repo 文件查看器是“原始文件预览器”，不是“结构化文档模型渲染器”。

BlockSuite 的优势在于：

- `PageEditor` 与 `EdgelessEditor` 共享同一底层 block tree。
- `edgeless` 可直接承载定位式 block，并使用 `xywh` 在画布定位。
- 自定义 block 和多视图能力强，适合统一白板与结构化内容模型。
- markdown 可导入为 block tree，也可导出为 markdown。

BlockSuite 不适合直接替换当前 raw markdown 文件预览的原因：

- 当前预览器天然面向 repo 文件语义，而不是面向 BlockSuite 文档语义。
- 当前预览器已经深度支持 HTML 清洗、图表代码块和本地文件路径解析。
- 把 repo markdown 一律转成 BlockSuite 文档会引入额外转换层，并模糊“原文件”和“画布文档”的边界。

因此，最合适的技术路线是：

- 白板重构到 BlockSuite。
- markdown 预览先嵌入为白板 block。
- repo 文件编辑器暂不替换。

## 目标

### 产品目标

- 将 `canvas` 视图重构为基于 `BlockSuite EdgelessEditor` 的白板工作台。
- 在同一白板中组织文件、markdown、图片、workflow 和 note。
- 让白板内容成为当前 session 对话的空间化上下文。
- 保留现有 composer、runtime session、attachment 和 Tauri 文件能力。
- 为后续 `PageEditor` 文档视图与结构化编辑扩展打下统一模型基础。

### 技术目标

- 去除对白板内核 `tldraw` 的依赖。
- 建立新的 BlockSuite 自定义 block 层。
- 前后端全链路统一 `block` 语义：SQLite、Tauri API、runtime metadata、frontend types 全部使用 `blockId` / `blockIds`。
- 白板内容直接持久化为 BlockSuite snapshot。
- 不依赖 `@affine/core` 应用层运行时。
- 只复用 BlockSuite 包和 AFFiNE / BlockSuite 仓库中的实现思路。

### 非目标

- 不处理旧 `tldraw` 画布导入。
- 不做多人实时协作。
- 不把 repo 文件编辑器改成 BlockSuite。
- 不引入 AFFiNE 的整套 peek-view、attachment-viewer、workbench 体系。
- 不在首阶段引入 Yjs 二进制 update 作为唯一持久化格式。

## 总体架构

### Session-first 模型

虽然 BlockSuite 天然围绕 `doc` 工作，但在 Sessio 里，白板仍然必须围绕 `session` 组织。

本方案采用明确的 `session-first` 模型：

- `session` 是产品主对象和页面主语义。
- `BlockSuite doc` 只是该 session 的白板状态容器。
- 白板上的 block 是 session 资源的空间化投影，而不是业务真相本身。

也就是说，在 Sessio 中我们不是“打开一个 BlockSuite 文档页面”，而是：

- 先打开一个 `session`
- 再在该 session 的 `canvas` 子视图中挂载一个 `BlockSuite EdgelessEditor`

因此，BlockSuite 在本项目里的定位应当是：

- 一个前端内容模型和白板渲染层
- 一个 session-scoped canvas state container

而不是：

- 独立知识库文档系统
- 页面级主业务对象

### Session 与 BlockSuite doc 的关系

推荐采用一对一绑定：

- 每个 session 对应一个 BlockSuite canvas doc
- 一个 canvas doc 只服务当前 session
- doc 生命周期跟随 session 视图与本地持久化策略

可将该关系理解为：

```text
Session
  ├─ chat transcript
  ├─ file view state
  ├─ composer state
  ├─ workflow/runtime context
  └─ canvas doc (BlockSuite)
```

这里的 `canvas doc` 不代表“用户文档实体”，仅代表：

- 当前 session 白板布局
- 当前 session 的 block 结构
- 当前 session 的 note / markdown preview / workflow card 状态

建议内部命名直接体现这种从属关系，例如：

- `session:{sessionId}:canvas`
- `sessionCanvasDocId`

而不是将其命名为独立通用的 document 概念。

### Session 仍然是业务事实源

以下能力继续由 session 驱动，而不是由 BlockSuite doc 驱动：

- 页面路由
- 当前运行中的 runtime session
- composer 发消息目标
- edited files 列表
- attachment 列表
- workflow / thread / stage 关联
- permission strips / working state
- anchor 与 context file 的上游归属

这意味着：

- `ChatPage` 继续是 session-first 页面
- `canvas` 只是 `ChatPage` 下的一个视图
- `BlockSuiteCanvasHost` 只消费 session 提供的资源和状态

### Block 是 session 资源的投影

白板上的 block 不直接拥有大多数业务真相，而是对 session 资源做可视化投影。

例如：

- `sessio:file-card` 指向 workspace 文件或 edited file
- `sessio:markdown-preview` 指向 markdown 文件，或保存一份白板侧只读 snapshot
- `sessio:workflow-card` 指向 thread / stage / workflow 定义
- `affine:note` 承载当前 session 白板里的自由批注

因此建议坚持以下原则：

- 文件内容真相仍在文件系统
- workflow 真相仍在 thread / stage / runtime 侧
- 白板只保存展示状态、布局和必要摘要

这样可以避免把 repo 文件、workflow 定义和白板状态混成一个文档系统。

### 推荐分层

建议把实现拆成四层：

#### 1. Session Layer

负责：

- `sessionId`
- `threadId`
- `workspacePath`
- composer
- runtime state
- edited files
- attachments
- workflow API

#### 2. Canvas Doc Layer

负责：

- `sessionId -> canvas doc` 的创建与加载
- BlockSuite snapshot 保存与恢复
- revision / autosave / 本地文件落盘

#### 3. Block Projection Layer

负责把 session 资源映射成 block：

- 文件 -> `sessio:file-card`
- markdown -> `sessio:markdown-preview`
- workflow -> `sessio:workflow-card`
- 图片 -> image / attachment block
- 自由笔记 -> `affine:note`

#### 4. Context Bridge Layer

负责把当前白板选区重新投影回 session 输入链路：

- 读取 selected block ids
- 解析对应 `sourcePath` / `workflow summary` / `note text`
- 生成 context file
- 注入 composer 与 attachment

### 为什么不采用“文档中心”做法

本方案明确不推荐在 v1 中采用以下模式：

- 把 session 白板做成多个 BlockSuite 文档互相嵌套引用的系统
- 把 repo 文件统一转成 BlockSuite 原生文档并作为白板主数据源
- 让 BlockSuite doc 成为高于 session 的顶层业务对象

原因是：

- Sessio 的主语义始终是 session，而不是知识库文档
- repo 文件需要保持原始文件视角和可编辑性
- workflow 需要继续挂靠现有 thread / runtime 模型
- v1 应优先把白板作为“空间化上下文层”，而不是新的内容数据库

### v1 的最稳实现方式

v1 推荐采用下面这条线：

- 每个 session 绑定一个 BlockSuite edgeless doc
- doc metadata 中保存 `sessionId`
- 白板中的大多数 block 只引用外部 session 资源
- 真正写进 BlockSuite doc 的内容只包括：
  - note
  - markdown preview 的只读展示状态
  - workflow 摘要
  - 布局与视图状态

这能带来几个直接收益：

- session 语义清晰
- 白板打开成本低
- repo 文件和白板状态边界明确
- 选区生成上下文更直接
- 后续可渐进引入 `PageEditor`，而不是一开始全量文档化

### 视图结构

保持现有 detail chat 页面信息架构不变：

```text
ChatPage
  ├─ Header / view toggle
  ├─ Main view
  │   ├─ chat
  │   ├─ file
  │   └─ canvas
  ├─ shared strips
  └─ SharedChatComposer
```

其中：

- `chat` 视图继续显示 transcript。
- `file` 视图继续显示 repo 文件。
- `canvas` 视图切换为 `BlockSuiteCanvasHost`。

### 运行时边界

#### WebView 前端职责

- 挂载 `BlockSuite EdgelessEditor`
- 渲染自定义 block
- 处理白板交互、选区、拖拽、缩放、block toolbar
- 收集当前白板选区上下文
- 调用已有 Tauri API 做保存、上下文文件生成、workflow 触发

#### Tauri / Rust 职责

- 文件读写
- 画布 revision 元数据持久化
- snapshot 文件落盘
- preview file watch
- 图片/附件读取
- workflow / session / thread 相关 API

#### 设计原则

- BlockSuite 只占据前端内容模型与渲染层。
- 业务主链路继续由 Sessio 自己掌控。
- 不把 AFFiNE 应用层的大量运行时模块带入 Sessio。

## 技术选型结论

### 选择 BlockSuite 的原因

- `EdgelessEditor` 与 `PageEditor` 共享文档模型。
- 支持在白板中承载定位式 block。
- 自定义 block、block view、toolbar extension 能力完整。
- 适合“白板 + 文档 + AI 上下文”的混合产品形态。

### 选择“自定义 block + 现有 markdown 渲染”而非“全文转 BlockSuite markdown 文档”的原因

- 可以复用 `PlainMarkdownPreview` / `PlainMarkdownPreviewContent` 的现有渲染能力。
- 能保留本地文件相对路径、图片、Mermaid、DOT、Vega、KaTeX 等行为。
- 白板内 markdown 卡片首先是“预览卡片”，不是“原始 repo 文件替代编辑器”。
- 降低首阶段重构风险。

### 不直接依赖 AFFiNE 应用层的原因

- `@affine/core` 中有大量与其工作台、存储、服务容器、peek-view 和 viewer 强绑定的模块。
- Sessio 只需要 BlockSuite SDK 能力，不需要把 AFFiNE 整个桌面应用 runtime 搬进来。
- 运行时依赖应保持最小化，避免后续升级困难。

### 包依赖策略

Phase 0 spike 可先使用 umbrella 或核心 SDK 包快速验证：

- `@blocksuite/affine`
- `@blocksuite/store`
- `@blocksuite/std`

但正式实现不应长期停留在“只靠 umbrella 包就够了”的假设上。`@blocksuite/affine` 实际会拉入大量 block / widget / gfx 依赖，因此：

- spike 通过后应立即收敛生产依赖清单
- 生产实现优先按实际需要选择子包、specs 和 extension 组合
- 只在确定需要时再引入 attachment / embed / surface-ref / database 等额外能力

生产实现至少要明确以下几类依赖：

- 文档与 schema：root / surface / note / image / frame / edgeless-text
- 视图基础设施：`@blocksuite/std`、store / view ext loader、必要 `effects`
- edgeless widgets：toolbar、selected rect、zoom toolbar、dragging area 等
- 主题与样式：BlockSuite / ToEverything 必需 CSS、字体和 effect 初始化

需要单独验证的技术风险：

- BlockSuite 包与 `React 19`
- BlockSuite 包与当前 `Vite`
- BlockSuite 包在当前 `Node >= 24` 开发环境下的安装和构建行为

因此第一个阶段必须先做 package spike，且 spike 的通过标准不能只停留在“空白 edgeless 能挂载”。

## 新数据模型

由于不做旧画布迁移，本次重构允许直接清理 `shape` 语义，建立新的 block 语义；同时直接修改 SQLite、Tauri command 和 runtime metadata，不保留兼容层。

### 文档范围

- 每个 session 对应一个新的 BlockSuite canvas document。
- 每个 canvas document 至少包含：
  - 一个 root page block
  - 一个 surface block
  - 零到多个 note block
  - 零到多个 surface child blocks
  - 零到多个 surface elements

### 白板上的内容类型

#### 使用原生 BlockSuite 能力的内容

- note：优先复用 `affine:note`
- image：优先复用 image / attachment 相关 block
- group / frame：优先使用 edgeless 原生机制

#### 使用 Sessio 自定义 block 的内容

- `sessio:markdown-preview`
- `sessio:file-card`
- `sessio:workflow-card`

### 全链路命名与协议边界

此次重构要求前后端统一替换以下概念：

- `CanvasShapeRef` -> `CanvasBlockRecord`
- `shapeId` -> `blockId`
- `kind` -> `blockKind`
- `selectionShapeIdsJson` -> `selectionBlockIdsJson`
- `anchorShapeId` -> `anchorBlockId`
- runtime metadata `shapeIds` -> `blockIds`
- `update_canvas_shape_refs` -> `update_canvas_blocks`

这里的边界要明确：

- `blockId` / `blockIds` 是新 canvas 协议里的主索引
- `blockKind` 是白板上下文桥接层使用的业务分类，不等同于 BlockSuite 内部原生 flavour 名
- `elementId` / `elementIds` 只作为 edgeless 选区补充定位信息，不替代 `blockId`

由于当前版本尚未 release：

- 不保留旧字段
- 不做 DB 迁移兼容层
- 不做 `shape` / `block` 双命名共存
- 同一分支内一次性完成 frontend types、SQLite schema、Tauri commands、runtime metadata 改名

补充约束：

- 这里的“一次性改名”针对对外协议、持久化字段和共享类型
- 旧 `tldraw` 宿主在下线前，内部直接调用 tldraw API 的局部变量名可暂时保留 `shape`
- 但这些旧局部命名不应再泄漏到新的 DTO、SQLite 字段、Tauri command 或 runtime metadata

### 新类型命名建议

新增独立类型文件，避免继续复用 `shape` 命名：

```ts
type CanvasBlockKind =
  | "markdown_preview"
  | "file_card"
  | "workflow_card"
  | "note"
  | "image"
  | "group";

type CanvasBlockSourceType =
  | "workspace_file"
  | "edited_file"
  | "attachment_image"
  | "workflow_definition"
  | "inline_markdown"
  | "note";

interface CanvasBlockRecord {
  id: string;
  canvasId: string;
  blockId: string;
  blockKind: CanvasBlockKind;
  sourceType: CanvasBlockSourceType;
  sourceKey?: string | null;
  sourcePath?: string | null;
  metadataJson: string;
}

interface CanvasAnchorInfo {
  id: string;
  canvasId: string;
  anchorBlockId: string | null;
  selectionBlockIdsJson: string;
  selectionElementIdsJson: string;
  turnId: string;
  summary: string | null;
  createdAt: number;
}

interface CanvasContextOption {
  canvasId: string;
  scope: "canvas" | "selection" | "anchor";
  blockIds: string[];
  elementIds: string[];
  anchorId?: string | null;
  snapshotAttachmentPath?: string | null;
  refs: CanvasContextRef[];
}
```

上下文、anchor、selection 等结构统一改用：

- `blockId`
- `blockKind`
- `sourceType`
- `elementIds` 仅作为可选补充定位信息

不再使用：

- `shapeId`
- `kind` 与 `shape.type` 混用

### 持久化策略

保留“本地文件存 snapshot + SQLite 存元数据索引”的模式，但 snapshot 改为 BlockSuite snapshot。

建议直接保留并重建以下概念：

- `CanvasDocumentInfo`
- `CanvasRevisionInfo`
- `CanvasBlockRecord`
- `CanvasAnchorInfo`

其中：

- BlockSuite snapshot 保存完整布局、block tree 和必要的 block props
- SQLite 保存 canvas document、revision、block index、anchor
- `CanvasBlockRecord` 是为 selection / context / inspector 服务的派生索引，不是业务真相
- block props 仍保存来源信息，但 selection / context / inspector 不应每次都全量遍历 snapshot 解析
- 如果后续确实需要更强搜索，可在 `CanvasBlockRecord` 之上再增加专用索引，而不是回退到 `shape` 语义

### 存储职责边界

这次重构继续采用“文件系统存原始白板数据，SQLite 存关系与索引”的双层模式。

文件系统负责保存：

- 当前 draft snapshot
- 已保存 revisions 的 snapshot
- 选区 / workflow 生成出来的 context 文件

SQLite 负责保存：

- `session -> canvas document` 的一对一关系
- 当前保存 revision 编号
- draft / revision 的路径、hash、size、时间戳
- block 索引记录
- anchor 与 selection 关系

也就是说：

- 文件系统中保存“白板真身”
- SQLite 中保存“白板目录、revision、block 索引、anchor、上下文关联”

### 本地文件布局

建议沿用当前 session-scoped canvas 目录布局，但文件名使用实现中性的 `canvas snapshot json`，不把底层库名直接写进路径：

```text
.canvas/
  <sessionId>/
    draft.canvas.json
    revisions/
      000001.canvas.json
      000002.canvas.json
      ...
    context/
      canvas-selection-<ts>.md
      workflow-<ts>.md
      ...
```

说明：

- `draft.canvas.json` 保存当前未手动 revision 的最新 snapshot
- `revisions/*.canvas.json` 保存手动或显式触发的已保存版本
- `context/` 保存由白板选区派生的 markdown context 文件，不直接写回 snapshot
- 文件写入继续使用原子写入策略：先写临时文件，再 rename

### Snapshot 文件内容

BlockSuite snapshot 文件应保存：

- root/page/surface 的结构
- 所有原生 block 与自定义 block 的 block tree
- edgeless 布局信息，如 `xywh`
- block 自身必要 props
- 必要的 view state，例如 collapse / scrollTop / renderMode

不应默认写入 snapshot 的内容：

- 全量 repo 文件正文副本
- workflow 的完整业务真相
- 运行时 session 状态
- composer 状态
- thread transcript

具体原则：

- repo 文件类 block 以 `sourcePath` 为主
- markdown 预览类 block 以 `sourcePath + excerpt + contentVersion + optional cachedContent` 为主
- workflow 类 block 以 `threadId / stageId / summary / latest execution pointer` 为主
- note 与纯白板内容直接写入 BlockSuite snapshot

### SQLite 表职责

建议直接将当前 `shape` 相关表改名并重建为 `block` 语义：

- `canvases`
- `canvas_revisions`
- `canvas_blocks`
- `canvas_context_anchors`

职责分别是：

- `canvases`
  - 一条记录对应一个 session 的 canvas document
  - 保存 `current_saved_revision`、draft 路径和 hash
- `canvas_revisions`
  - 每次手动保存生成一条 revision 记录
  - 保存 snapshot 路径、hash、size、source
- `canvas_blocks`
  - 保存从最新 snapshot 派生出的 block 索引
  - 字段至少包括 `block_id`、`block_kind`、`source_type`、`source_path`、`metadata_json`
- `canvas_context_anchors`
  - 保存 `anchorBlockId`
  - 保存 `selectionBlockIdsJson`
  - 保存 `selectionElementIdsJson`
  - 保存 `turnId` 与 `summary`

### 建议的 TypeScript DTO

```ts
interface CanvasDocumentInfo {
  id: string;
  sessionId: string;
  title: string;
  currentSavedRevision: number | null;
  draftSnapshotPath: string | null;
  draftSnapshotHash: string | null;
  draftUpdatedAt: number | null;
  createdAt: number;
  updatedAt: number;
}

interface CanvasRevisionInfo {
  id: string;
  canvasId: string;
  revision: number;
  snapshotPath: string;
  snapshotHash: string;
  snapshotSizeBytes: number;
  source: string;
  createdAt: number;
}

interface CanvasBlockRecord {
  id: string;
  canvasId: string;
  blockId: string;
  blockKind: CanvasBlockKind;
  sourceType: CanvasBlockSourceType;
  sourceKey: string | null;
  sourcePath: string | null;
  metadataJson: string;
  createdAt: number;
  updatedAt: number;
}

interface CanvasAnchorInfo {
  id: string;
  canvasId: string;
  anchorBlockId: string | null;
  selectionBlockIdsJson: string;
  selectionElementIdsJson: string;
  turnId: string;
  summary: string | null;
  createdAt: number;
}

interface CanvasDocumentState {
  document: CanvasDocumentInfo;
  draftSnapshot: string | null;
  savedRevision: CanvasRevisionInfo | null;
  savedSnapshot: string | null;
  blockRecords: CanvasBlockRecord[];
  anchors: CanvasAnchorInfo[];
}
```

### 建议的 Tauri Command DTO

```ts
interface SaveCanvasDraftRequest {
  sessionId: string;
  title?: string | null;
  snapshotJson: string;
}

interface SaveCanvasRevisionRequest {
  sessionId: string;
  title?: string | null;
  snapshotJson: string;
  source: string;
}

interface UpsertCanvasBlockRecordInput {
  blockId: string;
  blockKind: CanvasBlockKind;
  sourceType: CanvasBlockSourceType;
  sourceKey?: string | null;
  sourcePath?: string | null;
  metadataJson?: string | null;
}

interface UpdateCanvasBlocksRequest {
  sessionId: string;
  blocks: UpsertCanvasBlockRecordInput[];
}

interface UpsertCanvasAnchorRequest {
  sessionId: string;
  anchorBlockId?: string | null;
  selectionBlockIdsJson: string;
  selectionElementIdsJson: string;
  turnId: string;
  summary?: string | null;
}
```

### 写入时机

建议把写入时机固定为：

- editor 内容变更后 debounce autosave
  - 写 `draft.canvas.json`
  - 更新 `canvases.draft_snapshot_path/hash/updated_at`
  - 从当前 snapshot 派生 `canvas_blocks`
- 用户点击 Save 或触发显式保存
  - 生成新 revision 文件
  - 插入 `canvas_revisions`
  - 更新 `canvases.current_saved_revision`
  - 同步刷新 `canvas_blocks`
- 用户执行 Ask selection / workflow context
  - 写 `context/*.md`
  - 插入 `canvas_context_anchors`
- 用户恢复 revision
  - 读取 revision snapshot
  - 回写 draft
  - 刷新 `canvas_blocks`

### 为什么不把整份白板正文放进 SQLite

不建议把 snapshot 正文直接存入 SQLite，原因是：

- BlockSuite snapshot 体积会随 markdown preview、图片和布局增长
- revision 管理更适合文件系统
- 本地原子写文件和 hash 校验更直接
- context 文件、图片、附件本来就走文件系统路径

因此最稳的模型仍然是：

- JSON 文件负责内容
- SQLite 负责目录与索引

## 自定义 block 方案

### 实现骨架

每种 Sessio 自定义 block 至少包含以下层次：

- `store.ts`：注册 schema extension、props 和 default values
- `view.ts`：按 `page` / `edgeless` scope 注册 block view extension
- `effects.ts`：注册 custom element、portal 和必要 side effects
- `component.ts` 或 `*-edgeless.ts`：承载标题栏、选中态、scroll 容器和内容区
- `toolbar.ts` / `interaction.ts` / `clipboard.ts`：按需接入 toolbar、交互和复制粘贴
- `index.ts`：统一导出 store / view / effects 注册入口

也就是说，自定义 block 不是“一个 schema + 一个 React view”就能完成，而是要按 BlockSuite 的 store / view / effects 三层能力完整接入。

### 1. `sessio:markdown-preview`

用途：

- 在白板中显示 markdown 文件预览
- 可作为 repo markdown 文件的空间化预览卡片
- 可作为对话上下文来源

建议 props：

- `title`
- `sourcePath`
- `sourceType`
- `excerpt`
- `renderMode`
- `xywh`
- `collapsed`
- `scrollTop`
- `contentVersion`
- `cachedContent`

内容来源原则：

- repo 文件仍是 markdown 真相来源
- block 默认只持久化 `sourcePath`、`excerpt`、展示状态和 `contentVersion`
- `cachedContent` 仅作为可选缓存或离线 fallback，不作为默认真相来源
- 不在 v1 默认把完整 markdown 文件内容冗余进每个 block snapshot

渲染策略：

- 外层 block shell 负责标题栏、选中态、缩放态、滚动容器和 loading / empty 状态
- 默认渲染摘要模式，只展示标题、路径、excerpt 和必要状态
- 仅在选中、展开或显式“Open preview”时挂载完整 markdown body
- 完整预览优先复用 `PlainMarkdownPreviewContent`，而不是直接嵌入带自有 `ScrollArea` 的 `PlainMarkdownPreview`
- 缩放较小时自动退回摘要模式
- Mermaid / KaTeX / Vega / 图片等高成本内容采用惰性渲染或显式刷新策略

交互：

- 标题栏拖拽
- 内容区滚动
- 点击打开原文件
- 刷新文件内容
- 发送到对话上下文
- 摘要 / 完整预览切换

### 2. `sessio:file-card`

用途：

- 轻量展示 workspace 文件或最近编辑文件
- 不直接渲染完整 markdown 内容
- 作为白板组织、筛选和提问入口

建议 props：

- `title`
- `sourcePath`
- `sourceType`
- `subtitle`
- `summary`
- `xywh`
- `status`

交互：

- 打开右侧文件视图
- 转换为 markdown preview block
- 发送为上下文

### 3. `sessio:workflow-card`

用途：

- 承载 thread / stage workflow 的镜像摘要
- 可直接触发 workflow run
- 为对话上下文生成 workflow summary markdown

建议 props：

- `title`
- `threadId`
- `threadStageId`
- `sourceType`
- `workflowSummaryMarkdown`
- `executionState`
- `lastRunId`
- `xywh`

交互：

- 运行 workflow
- 打开 thread
- 生成 summary context file

### 4. Note 方案

自由文本批注优先直接复用原生 `affine:note`。

理由：

- note 已天然适配 page / edgeless
- 后续可与 `PageEditor` 共用内容模型
- 避免为了简单文本再造一个自定义 block
- 但仍需在 Sessio 侧补一层 projection / context adapter，把 note 文本接回对话上下文链路

## Markdown 集成策略

### 首阶段策略

保留现有文件编辑与预览双轨：

- repo 文件编辑：继续用 `PlainEditorView`
- repo 文件预览：继续用 `PlainMarkdownPreview`
- 白板中的 markdown：通过 `sessio:markdown-preview` 的 block shell + lazy `PlainMarkdownPreviewContent` 承载

### 后续结构化升级路径

Phase 5 后可新增“BlockSuite 文档视图”：

- 把 markdown 导入为 BlockSuite block tree
- 用 `PageEditor` 或只读 doc 视图展示
- 适用于 AI 生成文档、白板文档块、非 repo 原生文件场景

但这不是首阶段前提。

## 新模块结构建议

### 新增文件

建议新增：

- `src/components/blocksuite/BlockSuiteCanvasHost.tsx`
- `src/components/blocksuite/BlockSuiteCanvasShell.tsx`
- `src/lib/blocksuite/createCollection.ts`
- `src/lib/blocksuite/createEditor.ts`
- `src/lib/blocksuite/specs.ts`
- `src/lib/blocksuite/effects.ts`
- `src/lib/blocksuite/store-extensions.ts`
- `src/lib/blocksuite/view-extensions.ts`
- `src/lib/blocksuite/persistence.ts`
- `src/lib/blocksuite/context.ts`
- `src/lib/blocksuite/runtimeMetadata.ts`
- `src/lib/blocksuite/featureParity.ts`
- `src/lib/blocksuite/types.ts`
- `src/lib/blocksuite/blocks/markdown-preview/index.ts`
- `src/lib/blocksuite/blocks/markdown-preview/store.ts`
- `src/lib/blocksuite/blocks/markdown-preview/view.ts`
- `src/lib/blocksuite/blocks/markdown-preview/effects.ts`
- `src/lib/blocksuite/blocks/markdown-preview/component.ts`
- `src/lib/blocksuite/blocks/file-card/index.ts`
- `src/lib/blocksuite/blocks/file-card/store.ts`
- `src/lib/blocksuite/blocks/file-card/view.ts`
- `src/lib/blocksuite/blocks/file-card/effects.ts`
- `src/lib/blocksuite/blocks/file-card/component.ts`
- `src/lib/blocksuite/blocks/workflow-card/index.ts`
- `src/lib/blocksuite/blocks/workflow-card/store.ts`
- `src/lib/blocksuite/blocks/workflow-card/view.ts`
- `src/lib/blocksuite/blocks/workflow-card/effects.ts`
- `src/lib/blocksuite/blocks/workflow-card/component.ts`

### 需要修改的现有文件

- `src/pages/ChatPage.tsx`
- `src/App.tsx`
- `src/navigation.ts`
- `src/components/AppHeader.tsx`
- `src/api.ts`

可能保留但逐步替换的文件：

- `src/components/TldrawCanvasHost.tsx`
- `src/canvasTypes.ts`

## 白板上下文注入方案

当前 `tldraw` 方案的关键价值之一，是从白板选区生成对话上下文。

BlockSuite 重构后，这条链路保留，但从“读取选中的 tldraw shapes”改为“读取 selected block ids / selected element ids / selected refs”。

其中建议始终遵循：

- `selected block ids` 是上下文注入、anchor 和 inspector 的主输入
- `selected element ids` 只在需要保留 edgeless 选区细节时附带
- `selected refs` 是从 snapshot 与 `canvas_blocks` 联合投影出来的上下文摘要结构

### Context 组装原则

#### `markdown-preview`

- 生成来源文件路径
- 必要时生成 markdown context file
- summary 取标题、文件路径、可选摘要

#### `file-card`

- 生成来源路径
- 不必总是内联全文
- 根据提问路径决定是否转成 context file

#### `workflow-card`

- 生成 workflow summary markdown
- 附带 thread id / stage id / 运行态信息

#### `image`

- 走已有图片 attachment 逻辑

#### `note`

- 提取 note 的纯文本或 markdown

### Anchor 模型

保留当前 anchor 能力，但改用 block ids：

- `anchorBlockId`
- `selectionBlockIdsJson`
- `selectionElementIdsJson`
- `turnId`
- `summary`

其中：

- `anchorBlockId` 可为空，因为用户的选区不一定总能稳定投影到单一 block
- `selectionBlockIdsJson` 是 anchor 的主要索引字段
- `selectionElementIdsJson` 只在需要恢复更细粒度 edgeless 选区时使用

### Runtime / Tauri 协议直接改名

本次重构要求以下协议面一次性替换：

- `CanvasContextRef.shapeId` -> `CanvasContextRef.blockId`
- `CanvasContextOption.shapeIds` -> `CanvasContextOption.blockIds`
- `selectionShapeIdsJson` -> `selectionBlockIdsJson`
- `anchorShapeId` -> `anchorBlockId`
- `update_canvas_shape_refs` -> `update_canvas_blocks`

不保留新旧字段双写，也不保留临时兼容命令。

## 当前功能对齐基线

BlockSuite 版本在功能上至少需要对齐当前 `TldrawCanvasHost` 的这些能力：

- 添加文件、图片、workflow、note
- 批量添加 edited files
- draft autosave
- manual revision save
- restore last saved revision
- selection ask -> composer context
- selection snapshot attachment
- workflow run
- open linked thread chat
- inspector metadata panel
- recent anchors panel
- group / ungroup 或原生 edgeless 等价能力

## 具体实施阶段

### Phase 0：Package Spike

目标：

- 验证 BlockSuite 包可在 Sessio 当前前端环境中稳定安装和构建
- 验证最小 `affine-editor-container` 能在 React 页面中挂载
- 验证 theme CSS、字体、schema 注册和 `effects` 初始化路径
- 验证空白 `edgeless` 保存与恢复
- 验证至少一个 Sessio 自定义 block 的 store / view / effects 注册可跑通

输出：

- 一个最小 demo 组件
- 一个最小 snapshot 保存 / 读取流程
- 一个最小自定义 block demo
- 兼容性结论

验收：

- 页面中能看到空白 edgeless
- theme、字体和必要 effects 正常生效
- reload 后 snapshot 可恢复
- 自定义 block 保存 / 恢复可用
- 不引入明显构建阻塞

### Phase 1：Canvas Shell 替换

目标：

- 在 `ChatPage` 中新增 `BlockSuiteCanvasHost`
- 建立 canvas 生命周期、挂载、卸载、autosave 骨架
- 直接完成 canvas 数据契约从 `shape` 到 `block` 的重命名
- 接通 session-scoped document 与 revision 保存

输出：

- 基本空白白板
- 新的 `block` 命名类型、SQLite schema、Tauri API、runtime metadata
- 标准保存 / 手动保存 / 恢复逻辑

验收：

- `canvas` 视图可正常打开
- autosave 工作
- manual save 工作
- restore saved revision 工作
- 新 canvas 链路中不再暴露 `shapeId` / `shapeIds` / `CanvasShapeRef`

### Phase 2：Markdown Preview Block

目标：

- 实现 `sessio:markdown-preview`
- 在白板中显示 markdown 卡片
- 支持从 workspace 文件创建 markdown 卡片
- 建立摘要态 / 完整预览态双模式

输出：

- 标题栏拖拽
- 内容区滚动
- 文件刷新
- 打开原文件
- 默认摘要模式
- 按需挂载完整 preview body

验收：

- markdown 卡片在白板中可稳定显示
- 本地图片、Mermaid、KaTeX、Vega 行为与现有预览一致
- 低缩放或未聚焦时不会长期挂载完整重型 preview
- 白板缩放与卡片内部滚动职责分离清晰

### Phase 3：File / Workflow / Note 内容层

目标：

- 实现 `sessio:file-card`
- 实现 `sessio:workflow-card`
- 复用原生 note block
- 接通添加文件、添加 workflow、添加 note、添加图片
- 恢复 inspector、edited files picker、group / ungroup 等当前宿主能力

输出：

- 取代当前 `tldraw` 的主要内容入口
- 可用的白板侧 inspector / 操作入口

验收：

- 从编辑文件列表、workspace 文件、本地图片都能加入白板
- workflow 卡片能显示并保存状态
- workflow 卡片可打开 thread
- inspector 可查看 block 来源信息和状态
- group / ungroup 或原生等价能力可用

### Phase 4：上下文与对话集成

目标：

- 把当前白板选区变成 composer 的上下文来源
- 接通 anchor、context file、workflow summary、截图/选区对话
- 恢复当前 `TldrawCanvasHost` 的 selection ask / attach snapshot / workflow action 闭环

输出：

- selection summary
- anchor 保存
- context file 生成
- workflow run 触发
- selection snapshot attachment
- runtime metadata `blockIds`

验收：

- 从白板选区发问能进入当前 runtime session
- agent 能收到正确上下文
- attach selection snapshot 可用
- workflow run 与 open thread chat 可用
- anchor 面板可回看最近上下文锚点

### Phase 5：结构化文档层

目标：

- 引入 `PageEditor`
- 新增结构化文档视图或 AI 文档块
- 为将来“画布 + 文档”统一模型铺路

输出：

- 可选 doc block / doc preview 能力
- markdown transformer 在特定场景下可用

验收：

- 非 repo 文档内容可用 BlockSuite 原生文档承载

### Phase 6：收尾与清理

目标：

- 移除 `tldraw` 相关依赖与代码
- 清理 legacy canvas 类型和 API
- 完成性能、交互和样式收尾

输出：

- 新的 BlockSuite canvas 成为唯一实现

验收：

- 仓库里不再依赖 `tldraw`
- canvas 功能完整可用
- 当前功能对齐基线全部满足
- 新 canvas 代码与协议中不再残留 `shape` 命名

## 每阶段实施清单

### 优先级最高的首批任务

1. 建立 `src/components/blocksuite/BlockSuiteCanvasHost.tsx`
2. 建立最小 collection / editor / specs / effects 注册工具
3. 直接把 `canvasTypes`、SQLite schema、Tauri command、runtime metadata 改成 `block` 命名
4. 做最小 snapshot + revision 持久化
5. 做一个最小自定义 block 的 store / view / effects spike
6. 实现 `sessio:markdown-preview` 的摘要态壳层
7. 从文件面板接入 “Add to canvas as markdown block”

### 第二批任务

1. 实现 `sessio:file-card`
2. 实现 `sessio:workflow-card`
3. 接入 note / image / group
4. 恢复 inspector、edited files picker、recent anchors UI
5. 接入 context builder
6. 接入 selection snapshot attachment
7. 接入 workflow run / open thread

### 第三批任务

1. 引入 `PageEditor`
2. 统一 doc / canvas 数据结构命名
3. 移除 `tldraw`

## 风险与对策

### 1. BlockSuite 依赖接入复杂

风险：

- 包体较大
- 构建和 CSS 引入方式较重
- React 容器与 web components 生命周期需要磨合

对策：

- 先做 `Phase 0 spike`
- spike 期可先用 umbrella 包，生产实现尽快收敛到受控依赖清单
- React 只作为 host wrapper，不直接侵入 editor 内部

### 2. 自定义 block 接入比“schema + view”更复杂

风险：

- store / view / effects / customElements 任一层遗漏都可能造成渲染、反序列化或交互异常

对策：

- 每种 Sessio block 都按 `store.ts + view.ts + effects.ts + component.ts` 建骨架
- Phase 0 先做一个最小自定义 block spike
- Phase 2 之前不假设“直接包一个 React 组件”就能完成

### 3. 白板缩放与 markdown 内部滚动冲突

风险：

- 白板缩放手势与卡片内部滚动争抢

对策：

- 标题栏负责拖拽
- 内容区负责滚动
- 小尺寸下切为摘要卡

### 4. 大 markdown 卡片性能压力

风险：

- 长文档 + 图表 + 图片会让白板变重

对策：

- 卡片高度限制
- 低缩放摘要化
- 惰性图表渲染
- 默认不常驻完整 preview body
- 必要时使用显式刷新而非实时 watch

### 5. 继续沿用旧 `shape` 语义会污染新实现

风险：

- BlockSuite 实现里继续叫 `shapeId` 会让新旧模型混乱

对策：

- frontend types、SQLite schema、Tauri commands、runtime metadata 同一分支一次性切换到 `blockId`
- 旧 `tldraw` 代码只在下线前保留，不作为新方案接口来源
- 不做 `shape` / `block` 双命名兼容层

### 6. 当前宿主能力回归

风险：

- 重构后只保留“能显示白板”，却丢失当前 `TldrawCanvasHost` 已有的 revision、selection snapshot、workflow run、inspector、anchors 等能力

对策：

- 以“当前功能对齐基线”作为阶段验收前提
- 每个 Phase 验收都明确列出要恢复的宿主能力
- Phase 6 以前不宣告 `tldraw` 下线完成

## 验收标准

### 功能验收

- `canvas` 视图完全由 BlockSuite 驱动
- 能创建 markdown preview、file card、workflow card、note、image
- 白板内容可保存、恢复、手动保存
- 白板选区可生成对话上下文
- workflow 卡片可触发执行
- selection snapshot attachment 可用
- recent anchors / inspector / edited files picker 可用

### 交互验收

- 白板拖拽、缩放、选择稳定
- markdown 卡片可读、可滚动、可打开原文件
- markdown 卡片默认摘要化，按需进入完整预览
- 视图切换不影响 composer 和 session 状态

### 架构验收

- 不依赖 `@affine/core` 应用层 runtime
- 不再依赖 `tldraw`
- 新 canvas API、SQLite schema、Tauri command、runtime metadata 不再传播 `shape` 概念

## 最终建议

本次重构建议采用以下落地顺序：

1. 先做 package spike
2. 先锁定生产依赖 / specs / effects 组合
3. 直接把 canvas 数据契约从 `shape` 改成 `block`
4. 再把白板宿主替换成 BlockSuite
5. 第一优先级实现 markdown preview block shell
6. 再实现 file card、workflow card 和上下文链路
7. 最后引入 `PageEditor` 作为结构化文档能力

一句话总结：

> 用 BlockSuite 重构白板，但不要在首阶段重构 repo 文件编辑器；先完成 `shape -> block` 的全链路改名和 BlockSuite 宿主替换，再用“摘要态 block shell + 按需完整预览”接住 markdown 卡片，最后逐步扩展到统一的结构化文档体系。
