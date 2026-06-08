# New Chat Thread Mode Entry Plan

## 摘要

在 New Chat 输入框上方新增 thread 模式入口，支持 `workflow`、`teamwork`、`brainstorm`、`debate` 四种模式。用户选择模式后，New Chat 从普通会话入口变为 thread 创建入口：根据模式展示不同配置项，发送时创建 thread、启动第一条关联会话，并进入现有的 `ThreadChatPage`。

## 目标行为

- 未选择 thread 模式时，New Chat 保持现有普通 runtime session 行为。
- 选择 thread 模式后，输入框内容同时作为 thread goal 和首条用户消息。
- 按发送后先创建 thread，再启动首条 linked chat session。
- 成功启动后打开 `ThreadChatPage`，而不是跳到普通 `ChatPage`。
- 运行中的 session 仍通过现有 pending session / runtime alias 流程落库和索引。

## 模式配置

- `workflow`
  - 弹出当前 project 可用的 workflow/project stages。
  - 支持选择和排序 stages。
  - 创建 thread 后通过现有 `addThreadStage` 添加选中的 stages。
  - 不启用 Astra 自动调度，继续走人工定义阶段流程。

- `teamwork`
  - 弹出当前 project 可用的 assistants。
  - 使用现有 `thread_assistants` / `assistantIds` 创建 thread。
  - Astra teamwork 继续从 `thread.assistants` 路由任务。

- `brainstorm`
  - 弹出可用 runtime agent models。
  - 绑定 thread-level agent participants，而不是 assistants。
  - 每个 participant 保存完整运行配置：`agent`、`model`、`effort`、`permissionMode`、`order`。
  - 至少选择 1 个 participant。

- `debate`
  - 弹出可用 runtime agent models。
  - 绑定 thread-level agent participants，而不是 assistants。
  - 每个 participant 保存完整运行配置：`agent`、`model`、`effort`、`permissionMode`、`order`。
  - 至少选择 2 个 participants。

## 前端实施

- 修改 `src/pages/NewChatPage.tsx`
  - 在 `ChatComposer` 上方增加 mode segmented control。
  - 根据当前 mode 加载和展示 mode-specific selector。
  - project 切换时重置不再合法的 stage / assistant / participant 选择。
  - thread mode 发送时调用新 thread 创建流程；普通 mode 保持 `composer.runStartSession`。

- 复用现有组件和交互模式
  - stages 使用 `StageSelectChip` / drag ordering 的既有样式。
  - assistants 使用 `MultiPicker`。
  - agent model 选择复用 `agentModelSelectOptions`、`RuntimeMenuSelect`、effort / permission option helper。

- 修改 `src/components/AppMain.tsx`
  - 新增从 New Chat 创建 thread 后进入 `ThreadChatPage` 的回调。
  - 保持现有 `newChatSnapshot` 机制，用新 thread 作为 `snapshotContext.thread`。

- 修改 `src/navigation.ts`
  - 若需要，扩展 `PendingNewChatSession` 中的 thread metadata，确保首条会话能正确 link 到 thread 或 stage。

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
  - `debate` 少于 2 个 participants 时禁止发送或提示错误。
  - thread mode 发送后进入 `ThreadChatPage`。

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
- 首条输入既是 thread goal，也是首条 thread chat message。
- 新页面不从零实现，复用现有 `ThreadChatPage`。
