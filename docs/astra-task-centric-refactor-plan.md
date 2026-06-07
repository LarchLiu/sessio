# Astra Task-Centric Refactor / Teamwork 编排标准

## 摘要

本文档定义 Astra task-centric 编排的目标形态：**assistant 负责路由和上下文，task 负责执行事实，run 负责整体编排进度**。

它是 `teamwork` thread kind 的标准编排模型，不是 `workflow` 的自动调度模型。现有代码中 stage-based Astra 只是历史实现形态，本次重构要把它迁移为 assistant-routed teamwork：也就是把当前 Astra task 路由里的 stages 改为 thread-level assistants，由 Astra 根据 shared context 拆解 tasks、选择 assistants、调度执行、汇总结果，并决定下一轮或终态。

一句话边界：**`astra-task-centric-refactor-plan` 是 teamwork 的标准；它只把当前 Astra task-centric 流程里的 stages 改为 assistants，不把 workflow 改造成 Astra-scheduled workflow。**

因此本文档的“重构”含义非常窄：保留 Astra task-centric 编排能力，但把 routing/control plane 从 `targetStageId`、stage status 和 stage issue mutation，迁移到 `assistantId`、plan round/task lifecycle 和 task result。它不是把 `workflow` 变成 Astra-scheduled workflow，也不是给人为 stage 流程增加默认自动调度。

持久化模型以 `docs/thread-types-plan-rounds.md` 为准。在 teamwork 自动编排路径中，Astra run 负责编排进度、backend、diagnostics 和终态原因；`thread_plan_rounds` / `thread_plan_tasks` / `thread_plan_task_sessions` 负责每轮计划、task lifecycle、session 关联和 reload 恢复。本文档不再定义长期 `task_states_json` 或另一套 task state 事实源。

## 与 Thread Kind 的关系

四种 thread kind 的边界如下：

```text
workflow   = human-defined stages, no Astra scheduling
teamwork   = shared context + Astra task orchestration
brainstorm = shared context + parallel opinions + synthesis
PK/debate  = isolated contexts + cross-verification + convergence
```

- `workflow` 使用人为定义的 stages 和顺序。系统可以把人工 stage task 记录到 plan round/task，便于统一历史回看，但 workflow 不默认由 Astra 自动调度。
- `teamwork` 是本文档的范围：所有 assistants 共享 thread context，Astra 生成 plan round/tasks，并把 tasks 分派给 `assistantId` 或 `targetAgent`。它可以理解为“当前 Astra stages 改为 assistants”后的标准形态。
- `brainstorm` 不是普通 teamwork 的同义词。它需要 shared-board 生成、下一轮注入和 synthesis 策略。
- `debate` / PK 不是普通 teamwork 的同义词。它需要 isolated lanes、artifact 可见范围、cross-check 和 convergence 判断。

换句话说，旧的 stage-routed Astra 不是要沉淀为新的 workflow 调度器，而是要迁移为 teamwork 的 assistant-routed Astra。workflow 保留人工 stage 流程；teamwork 才拥有 Astra task orchestration。

## 当前问题

### 1. 旧 Astra 把 stage/issue mutation 交给 LLM

当前 prompt 要求 Astra 返回 `decisions`，并在其中表达 stage pass/fail/retry、issue open/resolve/dismiss、run complete/error 等状态变更。实际运行中已经出现多类问题：

- 返回结构不稳定，比如 JSON/YAML 混用、旧字段和新字段混用、列表解析 EOF。
- LLM 容易返回 stage id 作为 task id，或漏写 `approvedTaskIds` / issue 状态。
- 同一批并行 task 的聚合判断容易和单 task 判断混淆。
- 为了解析模型输出不断加兼容和补丁，但这会掩盖契约错误。

这说明 LLM 不适合做持久化状态 mutation 的 owner。LLM 更适合负责“下一步做什么”，Rust/store 更适合负责“已经发生了什么”和“run 是否终止”。

### 2. stage/status/issues 不是 teamwork 的控制面

现有模型里，`stage.status` 和 `issues` 同时用于 UI 展示、planner 过滤、prompt 上下文和下一轮决策输入。这会形成反馈回路：上一轮 LLM 生成的 issue 或 stage status，会被下一轮 LLM 当成事实继续推理。

在目标模型里：

- workflow stages 仍然保留给人为流程、归档和历史回看。
- teamwork 不读取 stage status，不写 stage/issue mutation。
- teamwork 的返工输入来自 prior task results、round summary、用户反馈和 explicit replan/retry，而不是 `thread_stage_issues`。

### 3. 并行 task running 状态不可可靠恢复

当前 UI 对 running task 的判断依赖 live event、`currentTaskId` 和 `approvedTaskIds` 推断。多个并行 task 刚开始时可以显示 running，但切换界面再回来后，只能从持久化 run 中恢复部分状态，导致只剩一个 task 显示 running。

根因是 task lifecycle 没有作为 thread-level 一等持久化状态存在。`currentTaskId` 是单值，不足以表达并行 task；live event 是瞬时信号，不能作为 reload 后的事实源。

## 目标架构

### 核心模型

```text
assistant = 路由 / 上下文 / agent 配置 / session 归档
task      = 执行事实 / 生命周期状态 / 结果记录
round     = 本轮计划 / dispatch mode / task batch
run       = 编排进度 / round cursor / 终态 / diagnostics
```

### assistant 负责路由，但不拥有 lifecycle

teamwork 的路由对象是 thread-level assistant：

- `assistantId` 表示产品成员和上下文角色。
- `targetAgent` 表示实际 runtime agent，可由 assistant 配置推出，也允许 agent-level task 没有 assistant。
- assistant 的 prompt、model、tools、runtime agent 等执行配置要在 task 创建时写入 `assistant_snapshot_json` / `agent_snapshot_json`。
- assistant 后续配置变化不能覆盖旧 task 的历史解释。

assistant 不表达 task lifecycle。planned/running/completed/failed/errored/cancelled 都由 `thread_plan_tasks.status` 表达。

### task 是执行事实源

task 至少支持：

- `planned`
- `running`
- `completed`
- `failed`
- `errored`
- `cancelled`

task 状态通过 `thread_plan_tasks.status` 持久化。`approvedTaskIds` 和 `currentTaskId` 从 run lifecycle contract 中删除，不参与 running 恢复。若后续需要人工 approval gate，应建模为 run/round 级 gate 或独立 action log，不作为 task lifecycle 状态。

并行 task dispatch 时，应一次性写入所有应启动 task 的 running 状态。reload 后，UI 应能只依赖 plan round/task 持久化数据恢复全部 running task；`listAstraRuns` 可以携带派生视图，但不能成为 task lifecycle 的唯一事实源。

### run 控制编排

run 负责：

- 当前 round index / round limit。
- active/terminal status。
- planner backend / diagnostics。
- terminal reason / error code。
- 是否继续生成下一轮、等待人工、完成或失败。

run 不读取 stage status 作为 blocked/needs_review/completed 控制条件。teamwork 下一步做什么由 Astra 根据 thread goal、thread assistants、shared context、历史 plan task results 和用户反馈生成。

## 新 Astra Contract

### 返回格式

Astra teamwork orchestrator 只返回一个完整 YAML document，不兼容 JSON，不接受 markdown code fence，不做 repair/fallback。这个 contract 只约束 teamwork backend；workflow 的人工 stage 流程、brainstorm 的 shared-board backend、debate 的 isolated-lane backend 不用它冒充完成。

```yaml
summary: string
runIntent: continue|complete|wait_for_human|error
reason: string
mode: parallel|sequential
tasks: []
```

### 字段语义

- `summary`：本轮规划摘要，用于 `thread_plan_rounds.summary`、run diagnostics 和 UI 展示。
- `runIntent`：
  - `continue`：继续自动编排，必须返回下一批 tasks。
  - `complete`：run 正常完成，`tasks` 必须为空。
  - `wait_for_human`：需要人工介入或评审，run 进入可诊断终态，`tasks` 必须为空。
  - `error`：不可恢复错误，run 进入 errored，`tasks` 必须为空。
- `reason`：解释 `runIntent`，用于 terminal reason 或 diagnostics。
- `mode`：`continue` 时必须存在，并写入 `thread_plan_rounds.mode`；terminal intent 时可忽略。
- `tasks`：下一批 task。`continue` 时不能为空；其他 intent 时必须为空。

### Task shape

```yaml
tasks:
  - title: string
    assistantId: assistant-id
    targetAgent: codex|claude|gemini|astra-pi
    prompt: string
    expectedOutput: string
    risk: low|medium|high
```

规则：

- `assistantId` 是 teamwork 的主要路由字段；没有 stage 时不能返回 `targetStageId`。
- `targetAgent` 表示实际 runtime agent。通常来自 assistant 配置，但允许显式覆盖。
- 允许 agent-level task 不带 `assistantId`，但 v1 UI 应优先鼓励绑定 assistant，便于团队成员语义和历史回看。
- `mode = parallel` 时，本轮 tasks 可以同时 dispatch。
- `mode = sequential` 时，本轮 tasks 按落库后的 `sort_order` 逐个 dispatch，且同一 round 任一时刻最多一个 running task。

### 明确删除的能力

LLM 不再返回以下内容：

- `decisions`
- `outcome`
- `issueAction`
- `stage mutation`
- `issue mutation`
- `approvedTaskIds`
- `currentTaskId`
- `targetStageId`
- `action/status/issueStatus` 等旧 decision shape

Rust 不做 JSON 兼容、不做 response repair、不做静默 fallback。格式错就是编排失败，并记录 raw response snippet 到 diagnostics。

## 实施顺序

全局实施顺序以 `docs/thread-types-plan-rounds.md` 为准：

1. `ThreadKind` / `thread_assistants` 先落地，让 thread 能表达 `workflow | teamwork | brainstorm | debate`，并能绑定 thread-level assistants。
2. 直接新增 `thread_plan_rounds` / `thread_plan_tasks` / `thread_plan_task_sessions`，建立 plan round 和 task lifecycle 的事实源。
3. 改 Astra 新 contract，让 planner 输出 `{ summary, runIntent, reason, mode, tasks }`，并把 plan round/tasks/session refs 写入上述表。
4. 接入 teamwork：使用本文档的 assistant-routed task-centric 编排，不要求 stages，不读 stage status，不写 stage/issue mutation。
5. 最后再做 brainstorm / debate；它们分别需要 shared-board backend 和 isolated-lane / cross-check backend，不能只靠普通 teamwork planner 宣称完成。

## 阶段计划

下面阶段按 `docs/thread-types-plan-rounds.md` 的全局实施顺序对齐。Phase 1/2 是 teamwork 编排的前置基础；真正改变 Astra contract 和调度行为从 Phase 3 开始。

### Phase 0: 直接替换旧 stage-decision 自动编排路径

目标：停止围绕旧 `decisions` contract 打兼容补丁，并把旧 stage-routed Astra 自动编排路径直接替换为 teamwork 的 assistant-routed plan-task contract。workflow 不再进入 Astra stage-decision 调度路径；旧 run lifecycle 存储字段和逻辑层直接删除，不做只读归档兼容。

执行内容：

- 明确后续持久化事实源以 `docs/thread-types-plan-rounds.md` 为准。
- 保留 300s timeout 和诊断增强方向。
- 不再新增 JSON repair、legacy decision shape 兼容或 pseudo function-call wrapper。
- 删除旧 stage-decision 自动调度入口；workflow thread 调用 Astra 时直接拒绝，不创建兼容 run。
- 从 Rust run record、SQLite DDL、Tauri API、TS API、UI 和 orchestrator 中删除 `proposedTasks/taskResults/currentTaskId/approvedTaskIds` 等旧 run lifecycle 字段。
- 旧 SQLite 表形状未 release，直接按新 schema 修改，不写旧 Astra run 存储兼容迁移。

验收：

- workflow 不再能启动 Astra 自动编排。
- brainstorm / debate 不通过旧 stage-decision 或 generic teamwork planner 兜底；只有接入专用 shared-board / isolated-lane backend 后才能启动。
- 旧 stage-decision response shape 被拒绝，不进入 fallback 或 repair。
- `AstraHandle` 不包含旧 run lifecycle 字段；task 展示从 plan round/task 查询。
- 后续代码改动按实施顺序拆分，不再混合 parser hotfix 和架构重构。

### Phase 1: 接入 ThreadKind 和 thread assistants

目标：先让 thread 能表达 `workflow | teamwork | brainstorm | debate`，并让 teamwork 有 thread-level assistants 作为路由对象。

执行内容：

- Rust 新增 `ThreadKind`，旧 thread 默认 `workflow`。
- `threads` / `ThreadInfo` / Tauri command / TS API 支持 thread kind。
- 新增 `thread_assistants`，用于绑定 teamwork/brainstorm/debate 的 thread-level assistants。
- ProjectPage 创建/编辑 thread 时可选择 kind 和 assistants。
- 本阶段不改变 Astra 自动编排行为；它只是为 teamwork routing 准备数据模型。

验收：

- 旧 thread 读取为 `workflow`，只表示历史数据可读，不表示保留旧 Astra stage-decision 自动调度兼容路径。
- 新建四种 thread kind 后 reload 仍正确。
- teamwork thread 可绑定多个 assistants。
- workflow 现有 stage 行为不变，且不因为 kind 落地而获得 Astra 默认调度。

### Phase 2: 接入 plan round/task/session 持久化

目标：让 plan round 和 task lifecycle 成为可 reload 的事实源，但暂不改变 Astra 编排 contract。

执行内容：

- 按 `docs/thread-types-plan-rounds.md` 新增 `thread_plan_rounds` / `thread_plan_tasks` / `thread_plan_task_sessions`。
- `thread_plan_rounds` 包含 `UNIQUE(thread_id, round_index)`、`astra_run_id` FK 和常用查询索引。
- `thread_plan_tasks` 保存 stage / assistant / agent 执行快照。
- `thread_plan_task_sessions` 保存 task 到 `(agent, session_id, role)` 的引用。
- 新增 create/list/get plan rounds with tasks、update task status、link/list task sessions 的 store API。
- sequential round 的“terminal 当前 task + start next task”必须在同一事务中完成，并保证同一 round 任一时刻最多一个 running task。
- 本阶段不让 Astra 新 contract 写表；它只建立共享事实源和执行不变量。

验收：

- 可以创建 parallel round 和 sequential round。
- parallel round 可同时存在多个 running task。
- sequential round 无法在 reload/并发下出现两个 running task。
- task result 可独立写入 terminal 状态、summary 和 error。
- 一个 task 可以关联多个 sessions。
- 删除 thread 时 plan rounds/tasks/task sessions cascade 删除。

### Phase 3: Astra 新 contract 写入 plan rounds/tasks

目标：把 teamwork Astra planner contract 改为 assistant-routed tasks-only YAML，并让新 run 写入 plan round/task/session 表。

执行内容：

- `AstraOrchestration` 改为 `{ summary, runIntent, reason, mode, tasks }`。
- 抽出公共 contract builder，runtime agent backend 和 Astra Pi ACP backend 共用。
- prompt 明确 teamwork 使用 `assistantId`，不返回 `targetStageId`。
- parser 只接受新 YAML shape。
- parser 使用 `deny_unknown_fields` 或等价严格校验，拒绝旧字段。
- 错误信息包含失败 code、简短 parser message、raw response snippet。
- `ASTRA_ORCHESTRATOR_TIMEOUT_MS` 只在一个位置定义为 `300_000`，所有 orchestrator backend 复用。
- planner 输出 task batch 后，先创建 `thread_plan_rounds` 和对应 `thread_plan_tasks`。
- 写入 teamwork task 时保存 assistant / agent 快照；workflow/manual task 的 stage 快照只服务历史回看，不作为 Astra 自动调度兼容入口。
- dispatch task batch 前，在同一事务中把本轮应启动的 plan tasks 更新为 `running`。
- dispatch/result 到达时，通过 `thread_plan_task_sessions` 记录 delegated/runtime session refs。
- result 到达时更新 plan task terminal 状态、result summary、error。
- `AstraHandle` 只暴露 run 元数据、backend、diagnostics 和终态信息；task lifecycle 展示必须从 plan round/task 查询，或由服务端基于 plan tables 显式派生。

验收：

- JSON response 被拒绝。
- code fence response 被拒绝。
- 旧 `decisions/action/status/issueStatus/targetStageId` response 被拒绝。
- workflow 创建 Astra run 直接失败。
- brainstorm / debate 不能通过旧 stage-decision 或 generic teamwork planner 创建 run；它们必须由 `docs/thread-types-plan-rounds.md` Phase 5/6 的专用 backend 接管。
- runtime agent 和 Astra Pi ACP 使用同一份 contract 文案。
- timeout 统一为 300s。
- Astra 每轮 plan 在 DB 中有 round 记录。
- 多 task 并行时所有 task running 可恢复。
- 每个 delegated/runtime session 都能从对应 plan task 反查。
- `currentTaskId` 不再影响并行 task 恢复正确性。

### Phase 4: Teamwork shared context 和 assistant routing

目标：让 teamwork thread 不需要 stages，也能由 Astra 自动拆解和执行。

执行内容：

- 主循环在 task batch 完成后调用 planner/backend 获取下一轮 tasks 或 terminal intent。
- Rust 根据 `runIntent` 处理 run 终态：
  - `complete` -> completed
  - `wait_for_human` -> completed 或 interrupted-like 可诊断终态，保留 reason
  - `error` -> errored
  - `continue` -> 创建下一轮 plan round/tasks 并按 mode dispatch
- Rust 根据 `mode` 和 plan task 状态执行 dispatch，不让 LLM 返回 running/completed mutation。
- 删除自动编排路径对 `AstraDecision::UpdateStage` / `AddOrUpdateIssue` / `RetryStage` 的依赖。
- Planner 输入包含 thread goal、thread-level assistants、assistant system prompts、历史 plan task results、用户反馈和 run diagnostics。
- Planner 不读取 stage status，不读取全局 stage issues。
- Dispatch 时按 `assistant_id` 找到 assistant 的 runtime agent 和 system prompt，并使用 task 快照构造 runtime prompt。
- 同 assistant retry/rework 时，prompt 带上相关 prior task result 和 rework reason，而不是 stage issue。
- Task result 回写 `thread_plan_tasks`，并驱动下一轮 plan。

验收：

- Teamwork thread 没有 stages 也能自动拆解和执行。
- 多 assistants parallel task 可同时运行并在 reload 后恢复。
- Sequential teamwork round 按 `sort_order` 执行。
- Teamwork 不读取 stage status，不写 stage/issue mutation。
- failed/errored/cancelled task 不需要 stage decision 也能进入下一轮、等待人工或终态。
- round status 从 task status 聚合。

### Phase 5: 前端和诊断收敛

目标：让 UI 与新事实源一致，并让错误可定位。

执行内容：

- Astra task card 状态从 `thread_plan_tasks` 或其派生视图读取。
- delegated session badge 从 `thread_plan_task_sessions` 展示。
- active run 面板展示 `runIntent` / terminal reason / last error code。
- parser failure、timeout、malformed response、backend session id 写入 run diagnostics。
- internal planner session 仍只进入 internal session ids，不进入普通 chat session 列表。

验收：

- reload 后 UI 和 plan task 持久化一致。
- “task result 跑到普通 chat”问题不复现。
- Astra planning session 不出现在普通 session 列表，只作为 internal diagnostics 可追踪。

### Phase 6: 删除旧 stage-decision 代码和旧测试

目标：清理旧控制面，避免未来继续误用。

执行内容：

- 删除或隔离旧 `AstraDecision` 自动编排路径；不需要为 workflow 保留旧 stage-decision 兼容入口。
- 删除旧 parser tests 中关于 update_stage/add_or_update_issue/retry_stage 的用例。
- 更新 deterministic backend，使其只输出新 `AstraOrchestration`。
- 清理 prompt 中所有 stage/issue mutation 和 `targetStageId` teamwork 说明。
- 保留 store API 和 CLI 的 stage/issue 人工操作能力，供 workflow/manual UI 使用。

验收：

- `cargo test --manifest-path src-tauri/Cargo.toml astra::` 通过。
- 前端 typecheck/build 通过。
- repo 中 Astra teamwork prompt 不再出现 `issueAction`、`update_stage`、`add_or_update_issue`、`targetStageId` 等旧 contract。

## 验收标准

### Parser / Contract

- 只接受 YAML mapping。
- 拒绝 JSON。
- 拒绝 markdown code fence。
- 拒绝旧 decision 字段。
- 拒绝 `targetStageId` 作为 teamwork routing。
- malformed response hard fail，不 fallback，不 repair。

### Teamwork Task

- task 使用 `assistantId` 或 agent-level `targetAgent` 路由。
- 同一 parallel round 多个 task 全部写入 running。
- 页面 reload 后全部 running 可恢复。
- 每个 task terminal result 独立更新，不影响其他 running task。
- sequential round 任一时刻最多一个 running task。

### Stage / Workflow

- Workflow 不默认由 Astra 自动调度。
- Teamwork 不读取 `stage.status` 作为 blocked/needs_review/completed 控制条件。
- Teamwork 不创建、关闭、dismiss stage issues。
- stage session 和 task session 继续保存，并能从 thread replay 聚合。

### Timeout / Diagnostics

- orchestrator timeout 统一为 300s。
- 不使用固定 sleep 处理数据流完成。
- timeout、parser failure、backend empty response 都写入 run diagnostics。

## 风险与取舍

### 风险 1: 旧 stage-based Astra 迁移面大

这次不是 parser hotfix，而是控制面重构。需要按实施顺序拆分：先落 `ThreadKind` / `thread_assistants`，再落 `thread_plan_rounds` / `thread_plan_tasks` / `thread_plan_task_sessions`，再改 Astra contract 写这些表，最后接入 teamwork/brainstorm/debate。

### 风险 2: 旧 run lifecycle 双轨风险

旧 run lifecycle 字段如果继续存在，会让 UI、orchestrator 和 reload 恢复出现双轨事实源。处理方式不是只读历史归档或派生展示，而是直接删除旧字段和旧逻辑层；新 run 必须使用 `thread_plan_tasks`。workflow 的旧 stage-decision 调度入口不需要兼容，直接由 teamwork assistant-routed contract 替换。

### 风险 3: workflow stage UI 与 teamwork Astra 分离

用户可能仍在 workflow UI 中看到 stage status 和 issues，但 teamwork Astra 自动编排不再受其控制。需要在实现和 UI 文案上避免暗示 stage status 会阻塞 teamwork。

### 风险 4: Brainstorm / Debate 不能只复用普通 teamwork

Teamwork 是 shared context + task orchestration。Brainstorm 还需要 shared board 和 synthesis；debate 还需要 isolated lanes、cross-check 和 convergence。后两者必须在 `docs/thread-types-plan-rounds.md` Phase 5/6 中通过专用 backend 或 v2 延后处理。

### 风险 5: 不 fallback 会暴露更多错误

严格契约会让 malformed response 直接失败，短期看错误会更明显。但这是有意选择：不要用 fallback 掩盖模型输出不符合协议的问题。诊断要足够清楚，让 prompt/backend 能被修正。

## 明确不做

- 不做旧 Astra run 存储兼容迁移；这些 SQLite schema 尚未 release，直接按新 schema 修改。
- 不删除 workflow stage/issue 人工 API。
- 不把 stage/issues 作为 teamwork prompt 控制输入。
- 不让 LLM 返回 stage/issue mutation。
- 不兼容 JSON。
- 不做 parser repair。
- 不做静默 deterministic fallback。
- 不用本文档的 teamwork contract 冒充 brainstorm / debate；两者按 `docs/thread-types-plan-rounds.md` 的专用 backend 语义实现。
