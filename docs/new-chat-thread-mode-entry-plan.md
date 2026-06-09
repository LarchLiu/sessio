# New Chat Thread Mode Entry Plan

## 摘要

在 New Chat 输入框底部第二个 selector 位置，将原 Kanban item 改为 thread kind 入口，支持 `workflow`、`teamwork`、`brainstorm`、`debate` 四种模式。用户选择模式后，New Chat 从普通会话入口变为 thread 创建入口：根据模式展示不同配置项，发送时创建 thread，并进入已新增的 `ThreadMultiSessionChatPage`。其中 `teamwork` / `brainstorm` / `debate` 会立即启动 Astra run；`workflow` 只创建人工 stage 编排，不启动 Astra 自动调度。

## 最新代码校准

- 当前代码已经有 `ThreadMultiSessionChatPage`，并通过 `DetailMode = "threadMultiSessionChat"` 路由：`AppMain` 在 `selectedThreadId && detailRoute === "threadMultiSessionChat"` 时渲染该页面。
- `PendingNewChatSession` 已有 `threadLink`、`workSnapshot`、`suppressAutoSelect`、`origin` 字段；该 pending 流程保留给普通 composer 启动的 thread/stage chat。New Chat thread mode 不再先创建 composer linked session，避免误用当前 composer 选中的 runtime agent。
- `NewChatPage` 已新增 thread kind selector、mode-specific setup panel、thread 创建流程；`Chat` 仍走普通 runtime session。
- thread API / store 已新增 `ThreadAgentInfo`、`agentParticipants`、`thread_agents`，用于 `brainstorm` / `debate` 的 thread-level agent participants。
- `ChatComposer` 已支持 `setupPanel` 和 `sendButtonVariant: "astra"`；thread mode 会把 primary send button 替换成 `ThreadMultiSessionChatPage` 里的 Astra `Sparkles` shimmer 样式。
- `ThreadChatPage` 仍存在，但它是旧的 snapshot/stage chat 入口。New Chat 的 thread mode 新流程不应再把它作为最终落点。

## 目标行为

- 未选择 thread 模式时，New Chat 保持现有普通 runtime session 行为。
- 选择 thread 模式后，输入框内容会先解析为 thread draft：第一行作为 `thread.goal`，剩余内容作为 `thread.description`。
- 按发送后先创建 thread；`teamwork` / `brainstorm` / `debate` 随即调用 `createAstraRun(thread.id, null)`，由 Astra 根据 thread assistants 或 agent participants 启动对应 multi-session lanes；`workflow` 不启动 Astra。
- 成功启动后打开 `ThreadMultiSessionChatPage`，而不是跳到普通 `ChatPage` 或旧的 `ThreadChatPage`。
- Astra 创建的 runtime sessions 通过 plan task / thread session link 进入 multi-session lanes；不经过 New Chat composer 的 pending linked session。

## 跳转规则

- mode 为空 / 普通 New Chat：调用 `composer.runStartSession`，保持现有自动选择逻辑，最终进入普通 `ChatPage`。
- mode 为 `workflow`、`teamwork`、`brainstorm`、`debate` 任意一种：创建 thread 后设置 `selectedThread` + `detailMode = "threadMultiSessionChat"`，最终进入 `ThreadMultiSessionChatPage`。`teamwork` / `brainstorm` / `debate` 在跳转前启动 Astra run。
- 这四种 thread mode 都不进入旧 `ThreadChatPage`；旧页面只服务现有 stage snapshot chat 路径。

## 输入框交互

- 不在输入框上方额外增加一排大按钮；保持 New Chat 当前紧凑输入框形态。
- 将底部第二个 selector 从 `No kanban item` 替换为 thread kind selector：
  - `Chat`：普通 New Chat，不创建 thread，最终进入 `ChatPage`。
  - `Teamwork`：创建 `teamwork` thread。
  - `Workflow`：创建 `workflow` thread。
  - `Brainstorm`：创建 `brainstorm` thread。
  - `Debate`：创建 `debate` thread。
- bottom row 推荐顺序：
  - project selector
  - thread kind selector
  - branch selector
- 选中 `Chat` 时不展示 thread 配置，保留普通发送体验。
- 选中四种 thread kind 时，在聊天框下方挂载轻量配置区；配置区宽度略短于聊天框，只保留下方两个圆角，并且不参与 New Chat 居中高度计算，避免切换 kind 或不同配置高度时让聊天框本体跳动。
- 配置区只展示当前 kind 需要的配置：
  - `Workflow`：stage chips + ordering。
  - `Teamwork`：project-level assistant chips，像 stages 一样直接显示和切换，默认全选当前 project 可用 assistants。
  - `Brainstorm`：global runtime agent participant list，不预设 agent，至少 2 个 participants。
  - `Debate`：global runtime agent participant lanes，不预设 agent，且只能选择 2 个 participants。
- 发送按钮按 mode 切换视觉语义：
  - `Chat`：保留当前圆形 `ArrowUp` 普通发送按钮。
  - `Workflow` / `Teamwork` / `Brainstorm` / `Debate`：把 primary send button 替换为 `ThreadMultiSessionChatPage` 里 Astra start button 的 `Sparkles` shimmer 样式，表示创建 thread 并进入 multi-session 页面；其中 `teamwork` / `brainstorm` / `debate` 同步启动 Astra run。
  - thread mode 发送中使用同位置 `LoaderCircle`，不要额外显示第二个 Astra 按钮。
  - thread mode tooltip/aria label 使用 `Create thread` 或更具体的 `Create workflow thread` / `Create debate thread`。
- Kanban item selector 从 New Chat 移除；Kanban 后续如仍需关联，可放到 Project/Workbench 或 session detail 的次级操作里，不作为 New Chat 主流程。

## 模式配置

- `workflow`
  - 弹出当前 project 可用的 workflow/project stages。
  - 支持选择和排序 stages。
  - 创建 thread 后通过现有 `addThreadStage` 添加选中的 stages。
  - 不启用 Astra 自动调度，继续走人工定义阶段流程。

- `teamwork`
  - 弹出当前 project 可用的 assistants。
  - 在 New Chat 中以直接可见 chips 展示，不使用下拉菜单。
  - 使用现有 `thread_assistants` / `assistantIds` 创建 thread。
  - Astra teamwork 继续从 `thread.assistants` 路由任务。

- `brainstorm`
  - 弹出全局可用 runtime agent models。
  - 绑定 thread-level agent participants，而不是 assistants。
  - 每个 participant 保存完整运行配置：`agent`、`model`、`effort`、`permissionMode`、`order`。
  - 不预设 participant，至少选择 2 个 participants。

- `debate`
  - 弹出全局可用 runtime agent models。
  - 绑定 thread-level agent participants，而不是 assistants。
  - 每个 participant 保存完整运行配置：`agent`、`model`、`effort`、`permissionMode`、`order`。
  - 不预设 participant，只能选择 2 个 participants。

## 前端实施

- 修改 `src/pages/NewChatPage.tsx`
  - 移除当前 bottom row 的 Kanban item selector。
  - 在原 Kanban item selector 位置新增 thread kind selector。
  - 根据当前 mode 加载和展示 mode-specific selector。
  - project 切换时重置不再合法的 stage / assistant / participant 选择。
  - thread mode 发送时调用新 thread 创建流程；未选择 thread mode 时保持 `composer.runStartSession` 并跳普通 `ChatPage`。
  - `teamwork` / `brainstorm` / `debate` 创建 thread 后调用 `createAstraRun`，不要再调用 `composer.runStartSession` 创建首条 linked session。
  - `workflow` 创建 thread 和 stages 后直接进入 `ThreadMultiSessionChatPage`，后续由用户在页面内手动启动 stage task。

- 修改 `src/components/ChatComposer.tsx`
  - 支持 primary send button 按 mode 替换样式，例如新增 `sendButtonVariant?: "chat" | "astra"` 或 `renderSendButton`。
  - `chat` variant 保持当前 `ArrowUp` 样式。
  - `astra` variant 复用 `ThreadMultiSessionChatPage` 的 Sparkles shimmer 按钮样式，busy 时显示 `LoaderCircle`。
  - 不通过 `sendActions` 增加第二个按钮；thread mode 是替换当前 primary send button。

- 复用现有组件和交互模式
  - stages 使用 `StageSelectChip` / drag ordering 的既有样式。
  - assistants 使用 `MultiPicker`。
  - agent model 选择复用 `agentModelSelectOptions`、`RuntimeMenuSelect`、effort / permission option helper。

- 修改 `src/components/AppMain.tsx`
  - 新增从 New Chat 创建 thread 后进入 `ThreadMultiSessionChatPage` 的回调。
  - 回调应设置 `selectedThread = { projectId, threadId, goal }`、清空普通 session selection，并设置 `detailMode = "threadMultiSessionChat"`。
  - 不要沿用 `newChatSnapshot` 作为这条新流程的主入口；`newChatSnapshot` 只保留给旧 `ThreadChatPage` / stage snapshot chat 路径。

- 修改 `src/navigation.ts`
  - 当前 `PendingNewChatSession` 已具备 `threadLink` / `workSnapshot` / `suppressAutoSelect` / `origin`；New Chat thread mode 不依赖这些字段。
  - 后续若新增手动 thread/stage chat 入口，再使用 pending metadata 确保 session 能正确 link 到 thread 或 stage。

## API 和存储实施

- 新增 thread-level agent participant 类型
  - TypeScript: `ThreadAgentInfo`
  - Rust: `ThreadAgentInfo`
  - 字段：`participantId`、`agent`、`model`、`effort`、`permissionMode`、`order`、`createdAt`、`updatedAt`

- 新增存储表，建议命名为 `thread_agents`
  - `thread_id`
  - `participant_id`
  - `agent`
  - `model`
  - `effort`
  - `permission_mode`
  - `sort_order`
  - `created_at`
  - `updated_at`
  - 删除 thread 时 cascade 删除 participants。

- 扩展 thread API
  - `ThreadInfo` 新增 `agentParticipants: ThreadAgentInfo[]`。
  - `createThread` / Tauri `create_thread` 新增可选 `agentParticipants`。
  - `updateThread` / Tauri `update_thread` 新增可选 `agentParticipants`。
  - `workflow`、`teamwork` 继续保留现有 `assistantIds` 行为。
  - `brainstorm`、`debate` 使用 `agentParticipants`，不再要求选择 assistants。

- 迁移
  - 当前 bootstrap schema 添加 `thread_agents`。
  - 增加新 schema migration 版本。
  - migration 应对已有库执行 `CREATE TABLE IF NOT EXISTS thread_agents ...` 和必要索引。

## Astra 路由实施

- `teamwork`
  - 保持现有 assistant-based routing。
  - `RuntimeAgentBackend`、Astra Pi adapter、planner prompt 中 teamwork contract 不变。

- `brainstorm`
  - `brainstorm_backend` 从 `thread.agentParticipants` 生成并行 tasks。
  - task 记录 participant id / agent runtime snapshot。
  - prompt 中展示 participant 的 agent/model/effort/permission 信息。

- `debate`
  - `debate_backend` 从 `thread.agentParticipants` 生成 isolated lanes。
  - lane id 基于 participant id。
  - cross-check visible artifacts 按 participant/lane 隔离。
  - 至少两个 participants，否则进入 wait-for-human / validation error。

- plan task snapshot
  - 保留现有 `targetAgent`。
  - 对 brainstorm/debate，`assistantSnapshotJson` 不再作为主要事实源。
  - 使用 `agentSnapshotJson` 保存 participant 的完整 runtime 配置。
  - UI 展示 plan task 时优先显示 participant snapshot；teamwork 仍显示 assistant snapshot。

## 测试计划

- 前端
  - New Chat 普通模式仍能启动普通会话。
  - `workflow` 模式能选择、排序 stages，并创建带 stages 的 thread。
  - `teamwork` 模式能选择 project assistants，并创建带 thread assistants 的 thread。
  - `brainstorm` 模式能选择 agent participants，并创建 agent-based thread。
  - `brainstorm` 少于 2 个 participants 时禁止发送或提示错误。
  - `debate` 不是 2 个 participants 时禁止发送或提示错误。
  - thread mode 发送后进入 `ThreadMultiSessionChatPage`。
  - `teamwork` / `brainstorm` / `debate` 不启动当前 composer agent；Astra 根据 thread assistants 或 selected agent participants 启动 lanes。
  - `debate` 创建后两个 selected agent participants 同时进入并行 lanes，UI 按两列展示。

- Rust/store
  - `thread_agents` create/list/update/delete round trip。
  - `ThreadInfo` 正确返回 `agentParticipants`。
  - 删除 thread cascade 删除 `thread_agents`。
  - `brainstorm` / `debate` 创建、读取、更新 agent participants。

- Astra
  - brainstorm backend 从 agent participants 生成 tasks。
  - debate backend 从 agent participants 生成 lanes。
  - debate 少于 2 participants 时不会开始无效编排。
  - plan task agent snapshots 保留 model / effort / permission。

- 验证命令
  - `pnpm run build`
  - relevant Rust store / Astra tests

## 默认和约束

- `workflow` 和 `teamwork` 不在本轮改成 agent-based。
- `brainstorm` / `debate` 不把 agent participant 偷偷写成 assistant。
- 首条输入的第一行是 thread goal，剩余内容是 thread description；完整输入仍作为首条 thread chat message。
- 最终落点复用已新增的 `ThreadMultiSessionChatPage`，不再跳转到旧 `ThreadChatPage`。
