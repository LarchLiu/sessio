# Plain View 接入 Notion-style (BlockNote) 可编辑器 — 改动计划

## 背景与目标

Chat page 当前的 `plain` 子视图(`FileViewer` 的 `mode="plain"` 分支)走
CodeMirror 只读 raw text 渲染路径,语义上仍是无结构、不可编辑的纯文本视图。目标是把
plain 替换为 **Notion-style 的 block 编辑器**(基于 BlockNote),并支持
**编辑写回磁盘**。

参考实现来自 `~/Work/cloudgeek/tolaria`(同一组织的笔记应用),其富文本编辑栈基于
BlockNote。本计划只移植**最小核心**,剥离 tolaria 与 vault 业务强绑定的部分。

### 已确认的需求决策

- **需要编辑写回**:用户可在 plain 视图内编辑工作区文件并保存到磁盘。
- **所有文本都走 BlockNote**:不区分 `.md` / 非 markdown,`.log` / `.txt` / 无扩展名
  的文本同样进入 BlockNote(非 markdown 内容由 BlockNote 的 markdown 解析尽力呈现)。

## 关键集成点(已探查)

| 关注点 | 现状 | 位置 |
|---|---|---|
| 视图类型 | `ChatView = "chat" \| "code" \| "plain"` | `src/navigation.ts:3` |
| 视图切换 UI | `ChatViewToggle`(三档) | `src/components/AppHeader.tsx` |
| files 路由 | `code`/`plain` 都归 files view,`filesSubview` 下传 | `src/pages/ChatPage.tsx:944` |
| plain 渲染 | `mode="plain"` 当前走 CodeMirror 只读 raw text | `src/components/FileViewer.tsx:243` |
| code view 能力边界 | 行号、语法高亮、git diff gutter/overview/preview 等归 code view,plain 首版不追求等价保留 | `src/components/FileViewer.tsx` |
| 文件读取 | `read_local_text_file`(上限 2MB) | `src-tauri/src/lib.rs:2987` |
| 文件写入 | **不存在**,需新增 | — |
| 文件内容 hook | `useFileContent` 负责 path 解析、cache、watch preview file、磁盘读取 | `src/hooks/useFileContent.ts` |
| agent 活跃状态 | `activeTurnId` 非空 = 有 turn 在跑 | `src/pages/ChatPage.tsx:644` |
| 主题 | `useEffectiveThemeType()` 读 `data-theme` | `src/components/shikiHighlight.tsx:188` |
| i18n | `useI18n()` / `t()`,`chat.files.*` 已有 | `src/i18n.tsx:302` |
| **CSP** | `csp: null`(**无限制**) | `src-tauri/tauri.conf.json:28` |

### 相对 tolaria 的简化

- **CSP 无需 nonce 注入**:sessio 的 CSP 为 `null`,而 tolaria 需要
  `_tiptapOptions: { injectNonce: RUNTIME_STYLE_NONCE }`。我们不需要。
- **不带 patches**:tolaria 的 `patches/@blocknote__*`、`prosemirror-tables` 全是
  wikilink/tldraw/code-block/link 定制,sessio 不需要。
- **不带自定义 schema**:tolaria 的 `editorSchema.tsx` 扩了 wikilink/math/mermaid/
  tldraw/audio/video。首版用 BlockNote **默认 schema**,后续按需单独移植。

## 渲染管线(目标)

```
ChatPage (isFilesView 路由, 不动)
  └─ ChatFilesView (picker/选择逻辑不动)
      └─ FileViewer  ← 改:mode="plain" 分支改用 NotionView
            └─ NotionView (新增, lazy 加载)        ← BlockNoteView 可编辑
                  └─ useNotionDoc (新增 hook)       ← md ⇄ blocks + 防抖写回
```

不碰 `code` 路径(Shiki 保留),不碰 toggle 档位(仍三档,只是 plain 渲染换实现)。

### plain / code 边界

- code view 继续承担源码阅读能力:行号、语法高亮、git diff gutter/overview/preview、
  原始文本滚动体验等。
- plain view 首版目标是「block 化阅读 + 可控写回」,不把 code view 的 diff/行号能力作为
  必须迁移项。需要看 diff 或逐行源码时,用户仍切回 code view。
- 因此 `FileViewer` 可以保留 code view 的 CodeMirror 实现,plain 分支单独切到
  `NotionView`。

## 风险点

1. **写回与 agent 写文件竞争**:agent 活跃(`activeTurnId` 非空)时锁定编辑器为只读。
2. **`blocksToMarkdownLossy` 有损**:frontmatter / HTML / 特殊 fence 等会被改写。
   首次解析后做 round-trip 比对,不一致则该文件**禁止编辑并提示**。
3. **mtime 冲突**:写回前校验磁盘 mtime,被外部(agent / 用户)动过则提示,不静默覆盖。
4. **包体积**:BlockNote + mantine 约 800KB–1MB,必须 `lazy()` 分包,不拖累 chat 首屏。
5. **主题对接**:把 BlockNote 的 `--bn-colors-*` 映射到 sessio 设计变量,
   用 `<MantineProvider>` 在编辑器边界裹住,隔离全局样式污染。
6. **解析失败/非 markdown 内容**:BlockNote 解析不出合法 block(抛错或解析为空)时,
   不应丢内容也不应回退到 `<pre>`,而是降级为 source-fallback(详见下方"大文件与解析回退")。
7. **文件系统越权**:files view 读/写命令必须限制在当前 `workspacePath` 内,不能只校验
   "绝对路径"。读写前用 canonical path 校验目标位于 workspace 下,symlink 逃逸也要拒绝。
8. **自动保存与 watcher/cache 回环**:当前 `useFileContent` 会监听文件变更并刷新缓存。
   BlockNote 自己保存后会触发同一 watcher,阶段 2 必须区分「本机刚保存」与「外部改动」,
   避免重灌 blocks 导致光标跳动或覆盖未保存编辑。
9. **依赖版本漂移**:`@blocknote/*@0.46` 是 tolaria 对齐版本,但当前 npm latest 已高于
   0.46。PoC 需明确是固定 0.46 还是升级到最新同 minor 组合;`@blocknote/mantine`
   的 peer 依赖还包含 `@mantine/hooks`,安装命令不能漏。
10. **大文件性能**:后端可允许读取 2–10MB 文本,但这不等于 BlockNote 对 10MB 文档可流畅
    交互。验收要分开验证「能读取」和「可编辑体验」,必要时对超大文件显示性能提示或只读。

## 大文件与解析回退(对齐 tolaria 做法)

**tolaria 不设前端大小阈值,也没有"大文件回退 `<pre>`"的概念**,所有文本内容统一进
BlockNote。它的回退是**按解析结果而非文件大小**触发的
(参考 `tolaria/src/hooks/editorMarkdownParseFallback.ts`):

- 正常:`tryParseMarkdownToBlocks(md)` → blocks。
- BlockNote **解析抛错**,或解析出**空 blocks 而源文非空** → 走 source-fallback:
  把源文按 `\n` 拆行,**每行生成一个 `paragraph` block**
  (`buildSourceLineBlock`:`{ type:'paragraph', content: line ? [{type:'text',text:line,styles:{}}] : [], children: [] }`),
  并标记 `usedSourceFallback: true`。
- 回退后内容**仍是 BlockNote 内可编辑的 block**,而**非** `<pre>`。
- source-fallback 不能直接用 `blocksToMarkdownLossy()` 保存,否则普通日志/文本可能被插入
  markdown 段落空行。阶段 2 若允许 fallback 文档编辑,必须配套 source-line serializer:
  将顶层 paragraph 的纯文本内容按原行重新 `join("\n")`;一旦用户插入无法线性保存的富 block,
  则提示并禁止保存。

对 sessio 的含义:

- **去掉**原方案里"大文件(>200KB)回退 `<pre>`"的设计。
- `.log` / `.txt` / 无扩展名 / 非法 markdown 一律进 BlockNote,解析不了就 source-fallback,
  保证「所有文本都走 BlockNote」且都可编辑写回。
- 上限由**后端**把守,但**不复用** attachment 预览的 `read_local_text_file`(其
  `MAX_TEXT_BYTES = 2MB` 是为附件预览设的,见下方"后端读写命令")。files view 走
  **独立命令 + 更大上限**,2MB 对编辑工作区文件太小。
- 在 sessio 移植一个 `parseMarkdownBlocksWithFallback` 等价工具(放
  `useNotionDoc.ts` 内或独立 util),封装 try/catch + 空结果判定 + source-fallback。
- 编辑开放条件需要区分:
  - markdown parse 模式:首次 parse 后用保存 serializer 做 round-trip 比对,不一致则只读。
  - source-fallback 模式:用 source-line serializer round-trip,一致才允许编辑保存。

## 后端读写命令(files view 专用,与 attachment 预览解耦)

**背景**:现有 `read_local_text_file`(`src-tauri/src/lib.rs:2987`)的 2MB 上限
(`MAX_TEXT_BYTES`)最初是为**聊天附件/资源块文本预览**引入的
(commit `68a97e4b` / `8b9a2655`,调用点 `ChatPage.tsx:2757`),后来被 files view 的
`useFileContent` 顺带复用。2MB 对「编辑工作区文件」场景太小,但**不应直接放宽
attachment 那条路径**(预览大附件没必要、且涉及内存)。

**做法**:为 files view 新增**独立**的读 / 写命令,各自的上限独立配置,并从阶段 1
就把 workspace 边界和 mtime 基准设计好。

- `read_workspace_text_file(workspace_path, path)` — files view 专用读取,上限
  `MAX_EDITOR_TEXT_BYTES`(建议 **10MB**,待确认),校验:绝对路径、is_file、text mime、
  canonical target 位于 canonical workspace 之下。返回 `{ content, mtime_ms }`,阶段 1
  先只消费 `content`,阶段 2 用 `mtime_ms` 做冲突基准。
- `write_workspace_text_file(workspace_path, path, content, expected_mtime_ms)` — 写回
  (阶段 2),同样以 `MAX_EDITOR_TEXT_BYTES` 限制 content 大小,同样做 workspace 边界校验。
  写前校验磁盘 mtime 与 `expected_mtime_ms` 一致;写成功后返回新的 `mtime_ms`。
- attachment 预览的 `read_local_text_file` **保持 2MB 不变**。
- 前端 `useFileContent`(files view 数据源)改调 `read_workspace_text_file`;
  attachment 预览的 `ChatPage.tsx:2757` 仍用 `read_local_text_file`。

> 上限值 `MAX_EDITOR_TEXT_BYTES` 建议 10MB:BlockNote 对超大文档性能有限,10MB
> 文本已属极端;若实际需要更大可再调。**此值需最终确认。**

---

## 分阶段实施方案

### 阶段 0 — PoC 验证(半天,不进主分支)

确认 BlockNote 在 sessio 的 Vite / Tauri 环境可正常运行,避免返工。

1. 装依赖:
   ```
   pnpm add @blocknote/core@^0.46 @blocknote/react@^0.46 @blocknote/mantine@^0.46 @mantine/core@^8 @mantine/hooks@^8
   ```
   版本策略需在 PoC 结论里写死:要么与 tolaria 对齐固定 `@blocknote/*@0.46.x`,
   要么整体升级到当前最新 `@blocknote/*` 同版本组合。不要混用不同 BlockNote minor。
   **不要**带 tolaria 的 `patches/`。
2. 临时挂一个最小 `BlockNoteView` 到 chat page,喂死字符串 markdown。
3. 验证三件事:
   - **样式渲染**:mantine 默认样式是否污染全局 sessio 样式。
   - **包体积**:lazy 分包后是否影响 chat 首屏。
   - **主题切换**:亮/暗模式是否正常。
   (CSP 已确认 null,无需验证 nonce。)

**门槛**:三项都过才进阶段 1。

### 阶段 1 — plain 接入 BlockNote(只读优先验证)(2–3 天,第一个可上线 PR)

目标:plain 视图显示为 block 渲染,**先只读**,零写回。后端仅新增读命令
(放宽上限),写回留到阶段 2。先把视觉/主题/体积坐实,再在阶段 2 打开编辑。

**后端(Rust,本阶段最小改动)**
- 新增 `read_workspace_text_file(workspace_path, path)`:files view 专用读取,上限
  `MAX_EDITOR_TEXT_BYTES`(建议 10MB,待确认),返回 `{ content, mtime_ms }`。
- 读命令从阶段 1 起就校验 canonical target 位于 canonical workspace 下,避免阶段 2
  写回时再临时补安全模型。
- 注册到 `invoke_handler`(`lib.rs:4059`)。attachment 的 `read_local_text_file`
  保持 2MB 不变。

**新增文件**
- `src/components/NotionView.tsx`(约 80 行)— `lazy` 加载,内部 `BlockNoteView`,
  阶段 1 设 `editable={false}`。
- `src/hooks/useNotionDoc.ts`(约 50 行)— 接收 `text`,经
  `parseMarkdownBlocksWithFallback`(try/catch + 空结果 source-fallback)解析为 blocks,
  `replaceBlocks` 灌入,按 `fileKey`(path + 内容 hash/mtime)失效重灌。
- `src/components/notion-theme.css` — `--bn-colors-*` 映射到 sessio 设计变量。

**改动文件**
- `api.ts`:加 `readWorkspaceTextFile` 包装;`useFileContent` 改调它(替换原
  `readLocalTextFile`),并让 `FileContentResult` 带上 `mtimeMs`;attachment 预览
  `ChatPage.tsx:2757` 不动。
- `FileViewer.tsx`:`mode === "plain"` 分支改为
  ```tsx
  <Suspense><NotionView fileKey={fileKey} text={text} /></Suspense>
  ```
  **不再**按大小回退 `<pre>`(大小由后端 `read_workspace_text_file` 把守)。
  解析失败/非 markdown 由 `useNotionDoc` 的 source-fallback 处理,仍进 BlockNote。

**验收**
- 打开 .md 文件,标题/列表/代码块/表格正确渲染。
- 非 markdown 文本(.log/.txt)也能进入 BlockNote 呈现。
- 解析失败/非法 markdown 触发 source-fallback,逐行进 BlockNote,不丢内容、不回退 `<pre>`。
- 切换文件内容随之刷新。
- 此前受 2MB 限制读不了的 2–10MB 文本文件现在能读取;同时记录 BlockNote 实际交互性能,
  不把「能读取」等同于「大文件可流畅编辑」。
- 亮/暗主题正常,样式不漏到 chat 区。
- 打包分析确认 BlockNote 在独立 chunk。

### 阶段 2 — 可编辑 + 写回磁盘(3–4 天,独立 PR,风险集中)

**后端(Rust)**
- 新增 `write_workspace_text_file(workspace_path, path, content, expected_mtime_ms)`(与阶段 1 的
  `read_workspace_text_file` 并列,紧邻现有 `read_local_text_file`,`lib.rs:2987`)。
- 路径必须绝对且 canonical target 在 canonical workspace 下;content 受
  `MAX_EDITOR_TEXT_BYTES` 限制;写前校验 mtime,磁盘 mtime ≠ `expected_mtime_ms`
  返回冲突错误,不覆盖。
- 写成功返回新的 `mtime_ms`,前端用它更新保存基准和 cache。
- 注册到 `invoke_handler`(`lib.rs:4059` 的列表)。
- TDD:先写失败测试,Rust 行覆盖维持(确认 sessio CI 实际门槛)。

**前端**
- `api.ts` 加 `writeWorkspaceTextFile` 包装(紧邻 `readLocalTextFile`,`api.ts:2091`)。
- `NotionView` 解除 `editable={false}`;`useNotionDoc` 增加 `editor.onChange` →
  防抖 500ms → 选择 serializer(markdown: `blocksToMarkdownLossy`;source-fallback:
  source-line serializer) → 写回。
- **round-trip 守卫**:首次解析后立即用对应 serializer 比对原文(需明确处理 CRLF/LF、
  trailing newline 的策略),不一致则该文件强制只读 + 提示(有损警告)。
- **agent 活跃锁**:`activeTurnId` 非空时 `editable={false}` + 提示
  「agent 运行中,暂不可编辑」。实现上需把状态从
  `ChatPage -> ChatFilesView -> FileViewer/NotionView` 下传。
- 保存失败 / mtime 冲突显示 banner(复用 `useFileContent` 的 `diskMayBeStale`
  提示样式)。
- **watcher/cache 回环处理**:本机保存成功后更新 `mtimeMs` 与 cache,并忽略/吸收对应的
  watcher 事件;若 watcher 事件来自外部改动且当前有 pending dirty change,显示冲突提示,
  不自动重灌 blocks。
- 新增 i18n 文案(`chat.files.*` 或新前缀),en + zh 双语(`src/i18n.tsx`)。

**验收**
- 编辑 → 自动保存 → 磁盘内容更新。
- 编辑期间 agent 启动 → 编辑器锁定。
- 外部改动后再保存 → mtime 冲突提示,不静默覆盖。
- 有损文件(frontmatter、HTML、特殊 fence、GFM 边界、CRLF/trailing newline 策略不一致)
  → 自动只读 + 提示。
- 本机保存触发 watcher 时不重置光标/选择区;外部改动才提示或刷新。

### 阶段 3 — 富 block 增强(可选,按需)

默认 schema 起步即可。若要 tolaria 那种 math / mermaid:从 `editorSchema.tsx` 单独
移植对应 spec + 各自的 markdown 适配器(`mathMarkdown.ts` / `mermaidMarkdown.ts`),
每个都是干净独立模块。wikilink / tldraw 与 vault 强绑定,**不要**带。

---

## 依赖与命令

```
pnpm add @blocknote/core@^0.46 @blocknote/react@^0.46 @blocknote/mantine@^0.46 @mantine/core@^8 @mantine/hooks@^8
```

CSS 需 import:`@blocknote/mantine/style.css`(在 `NotionView` 内,随 lazy chunk)。

## 待定/需确认

- `MAX_EDITOR_TEXT_BYTES` 的具体值(建议 10MB)。
- sessio 的前端/Rust 覆盖率门槛具体数值(确认 sessio CI)。
- BlockNote 版本策略:固定 tolaria 对齐的 `0.46.x`,还是升到当前最新同版本组合。
- round-trip 比对是否做换行归一化(CRLF/LF、末尾 newline),以及提示文案如何解释。
