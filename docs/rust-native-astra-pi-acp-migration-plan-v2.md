# Rust Native Astra + Pi ACP 重构实施方案 (v2)

## 修订说明 (v2 相对 v1)

本版在 v1 基础上修订，重点补强**异常路径的终态保证**与**输入边界的收敛语义**——这是 orchestrator 类系统最容易产生 orphan / 卡死的地方。相对 v1 的关键变化：

1. 新增「当前实现状态」小节，明确 Phase 1 主体已在 Rust 落地，避免把已实现部分误读为待实施。
2. 补强 worker 顶层错误必须落地为 terminal status，并增加运行期僵尸 run 检测（不只是启动时 interrupt）。
3. 定义 `needs_review` 的收敛终态，避免空耗 round limit 后被误判 errored。
4. 明确 `retry_stage` 与 `plan_next_round` 在 orchestrator 的差异化处理（为 Pi 接入预留语义）。
5. 补充「无可委派 agent 的 stage」「空 stages thread」的可诊断终态。
6. 补充 interrupted run 残留 partial 占位 session 的清理策略。
7. 把持久化 schema 扩展显式排进阶段计划（Phase 1 与 Phase 2 之间）。
8. 区分 timeout 分层的「目标态」与「各 Phase 现状」。
9. 标注过渡期 sidecar 路径的 apiKey 明文风险。
10. 明确 rust-native 路径的人工审批语义（默认全自动，`AwaitingApproval` 暂不进入）。

## 摘要

本次重构采用双轨迁移，目标是在不阻断现有能力的前提下，把 Astra 从“TS sidecar 负责策划/决策 + Rust 负责执行 + 私有 RPC 桥接”收敛为“Rust 单进程 orchestrator + 统一 ACP runtime + `pi_agent_rust` 作为 planner/decision backend”。

本方案按阶段实施，且允许在中期重构前端/Tauri API，不强制保持当前 `start_thread_astra` / `thread-astra-event` 形状不变。实施结束后，`sidecars/sessio-astra`、Astra 私有协议、`tool/call` 回调桥都将删除，只保留 Sessio Rust 与各 agent 的 ACP 交互。

## 当前实现状态

本方案是「现状 + 前瞻」的混合，落地时必须区分以下三类，避免重复实现或漏实现：

- **已落地（Phase 1 主体）**：
  - `src-tauri/src/astra/{planner,decision,prompt}.rs` 模块拆分。
  - `AstraPlanner` / `DeterministicPlanner`、`AstraDecisionEngine` / `DeterministicDecisionEngine` trait 与实现。
  - `run_rust_native_orchestrator` 主循环、`AstraWorkerGuard` worker registry、retry limit、round limit、deterministic stage/issue mutation。
  - 启动时 `recover_interrupted_runs` 把 active run 标记 `interrupted`。
  - delegated session 在 session start / agent session id 可持久化时即时 linking。
  - `legacy_sidecar` 通过 `SESSIO_ASTRA_LEGACY_SIDECAR` env 显式开关，默认 `rust_native`。
- **方案已写但代码未落地（需排期）**：
  - 持久化字段扩展（见「持久化与事件模型」）。
  - 可配置 round limit / retry limit（当前 `RUST_NATIVE_ROUND_LIMIT` 为硬编码常量）。
  - 本版新增的异常终态保证与边界收敛（见对应小节，标注「v2 新增」）。
- **后续 Phase（未开始）**：
  - Phase 2 `PiAcpPlanner` / `PiAcpDecisionEngine` 与 internal ACP backend。
  - Phase 3 前端/Tauri API 收敛与私有 RPC 删除。
  - Phase 4 sidecar 与打包残留删除。

## 目标架构

```text
Frontend
  -> Tauri commands/events
Rust Astra Orchestrator
  -> AstraPlanner trait
    -> DeterministicPlanner
    -> PiAcpPlanner
  -> AstraDecisionEngine trait
    -> DeterministicDecisionEngine
    -> PiAcpDecisionEngine
  -> RuntimeManager / AcpTransport
    -> pi_agent_rust (internal planning/decision session)
    -> codex / claude / gemini (delegated task sessions)
Store / Memory / Thread state
  -> Rust direct calls
```

## 关键接口与行为变更

- 新增 Rust 内部 `AstraPlanner` trait，planner 只负责根据 thread/stage snapshot 生成 `AstraPlan`，不负责 task dispatch 或直接 mutation。
- 新增 Rust 内部 `AstraDecisionEngine` trait，decision engine 负责根据 delegated task result、最新 snapshot、retry 状态生成 `AstraStageDecision` / `AstraIssueDecision` / retry / complete 等“请求型决策”，不直接写 store。
- 新增 Rust 内部 `AstraOrchestrator`，统一负责 run lifecycle、planning rounds、cancel、retry、task dispatch、result recording、决策校验与 stage/issue mutation 执行。
- Rust store 仍是 run、stage、issue、session linking 的唯一 durable owner；Pi 与 delegated agent 只能返回计划或决策请求。
- 规划器后端分为两种：
  - `DeterministicPlanner`：无 Pi 时的稳定 fallback。
  - `PiAcpPlanner`：通过内部 ACP backend 向 `pi_agent_rust` 发起短生命周期 planning session。
- 决策后端分为两种：
  - `DeterministicDecisionEngine`：保守更新，只在明确成功/失败信号下推进 stage 或创建 issue。
  - `PiAcpDecisionEngine`：通过内部 ACP backend 向 `pi_agent_rust` 发起短生命周期 decision session。
- `pi_agent_rust` 必须被建模为内部 ACP backend，而不是 Codex/Claude/Gemini 同类用户 agent；它不进入历史 agent enum，不出现在普通 chat runtime 列表，不参与 indexed session 语义。
- 前端/public API 允许收敛为更清晰的形状：
  - 保留“启动 run / 取消 run / 读取 runs / 订阅事件”四类能力。
  - 允许重命名命令和事件，但必须一次性完成迁移，不保留双命名长期兼容层。
- Astra 不再拥有独立 sidecar 协议；所有 agent 交互统一走 runtime layer。

### 人工审批语义（v2 明确）

- rust-native 路径默认**全自动**：`Planning -> Dispatching -> Running -> (Completed | ...)`，不进入人工审批环节。
- `AstraRunStatus::AwaitingApproval` 与 `approved_task_ids` 字段是 legacy sidecar 的遗留概念，rust-native 路径**当前不进入**该状态，但保留枚举值以兼容历史 run record 反序列化。
- 若产品后续需要「task 提案需用户确认才 dispatch」，必须作为**显式新增能力**进入独立 Phase，并补齐：审批超时、审批被拒后的 run 终态、审批期间的 cancel 行为。本方案范围内不实现人工审批。

## 新领域契约

Rust-native Astra 必须把“生成任务”“判断结果”“执行状态变更”拆成三个边界，避免把 sidecar 的隐式智能迁移成 Rust 内部黑盒。

### `AstraPlanner`

职责：

- 输入 run 元数据、thread/stage snapshot、user prompt、可选历史 task results、round index。
- 输出 `AstraPlan { summary, tasks[] }`。
- 不 dispatch task。
- 不写 store。
- 不声称 stage/issue 已变更。

输出约束：

- `tasks[].targetStageId` 必须解析到当前 thread stage，或为 `null` 表示 thread-level task。
- `tasks[].targetAgent` 必须是当前可委派 agent，且优先匹配 stage assistants。
- 单轮 task 数必须有上限，默认不超过 20。
- task id 由 Rust sanitize/normalize 后生成或去重，不能信任 planner 原始 id。

输入边界（v2 新增）：

- **无可委派 agent 的 stage**：`pick_stage_agent` 对未配置 assistant 的 stage 返回 `None`，planner 当前直接将其过滤掉。当 thread 中所有非终态 stage 都无 assistant 时，会出现「无 dispatchable task 且非全终态」，必须收敛为专门的可诊断终态（见 Orchestrator「无 dispatchable tasks 的分类收敛」），而不是笼统的 planner 失败。
- **空 stages thread**：`thread_all_stages_terminal` 在 `stages.is_empty()` 时返回 `false`，因此空 thread 不会被判为完成。必须为「thread 无可编排 stage」给出明确终态（建议直接 `Completed` 并附 `terminalReason = "no_stages_to_orchestrate"`，或专门的 error code），而不是落到通用 errored 分支。

### `AstraDecisionEngine`

职责：

- 输入 latest snapshot、刚完成的 `AstraTaskResult`、stage attempt counts、retry limit、历史 decisions。
- 输出 `AstraDecision`：
  - `UpdateStage`
  - `AddOrUpdateIssue`
  - `RetryStage`
  - `PlanNextRound`
  - `CompleteRun`
  - `ErrorRun`
  - （内部）`CancelRun`、`Composite`：用于 cancelled 结果与「issue + blocked」的组合决策。
- 不直接写 store。
- 不绕过 retry limit。
- 不创建 delegated runtime session。

设计理由：

- 现有 sidecar 通过 `sessio.stage.update` / `sessio.stage.issue.add_or_update` 承担结果判断。如果迁移后只有 planner 产出 tasks，Rust orchestrator 会缺少“结果是否足以完成 stage”的语义来源。
- 因此 Phase 1 必须同时落地 deterministic decision engine，Phase 2 再把 Pi ACP decision engine 接入同一契约。

`needs_review` 收敛终态（v2 新增）：

- deterministic decision 对「completed 但无明确完成信号」的输出会把 stage 置为 `needs_review`。`needs_review` **不是** planner 的终态过滤条件（planner 只跳过 `Completed | Skipped`），因此在不加约束的情况下，该 stage 会被逐轮重新 advance，最终撞上 round limit 被整体标记 errored——这是错误的：stage 实际可能已完成，只是缺显式信号。
- 必须显式定义 `needs_review` 的收敛规则，二选一并固定下来：
  - **方案 A（推荐）**：planner 把已是 `needs_review` 的 stage 视为「不可自动推进」，不再为其生成 advance task；当所有非终态 stage 都处于 `needs_review`/无可推进任务时，run 收敛为 `Completed`（附 `terminalReason = "pending_human_review"`），把后续判断交给人工。
  - **方案 B**：decision engine 对同一 stage 连续 N 次产出 `needs_review` 后，转为 `AddOrUpdateIssue`（提示人工复核）并把 stage 标记为非阻塞的待审状态，run 正常结束。
- 无论选哪种，`needs_review` 都**不得**沿用「重复重试直到 round limit → errored」的路径。

`RetryStage` vs `PlanNextRound` 语义区分（v2 新增）：

- 当前 deterministic 实现把两者都映射为「继续下一轮」，在 deterministic planner 下无害（下一轮全量重扫 stage）。
- 但二者语义不同，Phase 2 Pi decision engine 接入后必须区分：
  - `RetryStage`：对**同一 stage / 同一 task** 立即重试（在 retry limit 内），不等下一轮重新规划。
  - `PlanNextRound`：丢弃本轮剩余、触发 planner 重新生成整轮 tasks。
- orchestrator 必须为这两个 action 提供不同的控制流分支，并在持久化层记录区分（便于诊断 Pi 的重试行为）。

### `AstraOrchestrator`

职责：

- 是唯一 run lifecycle owner。
- 串行推进同一 run 的状态变更。
- 负责 active-run guard、round limit、retry limit、cancel token、timeout。
- 调用 planner 获取下一轮 task。
- 调用 runtime dispatch delegated task。
- 调用 decision engine 判断 task result。
- 校验 decision，再通过 store API 执行 stage/issue mutation。
- 在 delegated session 创建并拿到可持久化 agent/runtime session id 时立即记录 delegated session linking；terminal result 到达时只补 task result、attempt outcome、round history、terminal reason。

不允许：

- planner 或 decision engine 直接写 store。
- delegated agent 直接写 Astra run 状态。
- `pi_agent_rust` 直接 dispatch Codex/Claude/Gemini。
- 任意 backend 使用 shell/CLI 绕过 Rust store mutation。

worker 异常终态保证（v2 新增，**高优先级**）：

- orchestrator worker 在后台线程运行，其顶层 `Result` 的 `Err` 当前只被 `log::warn!`，**run status 不会更新**——任何内部 `?` 提前返回（`load_run`、`get_thread_work_state`、`mutate_run` 失败等）都会让 run 永久停在 `Planning`/`Dispatching`/`Running`。
- 必须保证：worker 闭包捕获顶层 `Err` 时，调用 `fail_run`（或等价的 `update_active_status -> Errored`）把 run 落到 terminal，并 emit `error` 事件，附 `lastErrorCode` / `lastErrorMessage`。
- worker panic 也必须被兜底（`std::panic::catch_unwind` 或在 `AstraWorkerGuard::drop` 中检测「registry 已清理但 run 仍 active」并补一个 errored），保证 guard 退出时 run 不会停留在 active。

运行期僵尸 run 检测（v2 新增）：

- 仅靠启动时 `recover_interrupted_runs` 不足以覆盖「进程存活但 worker 已死」的情况。重复 `start_thread_astra` 命中 active run 时会直接返回该 run 且不重启 worker，若 worker 已死则 run 永久卡住。
- 必须增加运行期检测：`start_thread_astra` 命中 active run 时，校验该 run 是否仍在 `orchestrator_workers` registry 中；若 status active 但无对应 worker，则视为僵尸，将其标记 `interrupted`（附 reason）后允许重新创建/接管。

无 dispatchable tasks 的分类收敛（v2 新增）：

- 当 planner 产出空 dispatchable 集合时，必须按原因分类终态，而非统一 errored：
  - 全部 stage 终态（`Completed`/`Skipped`）→ `Completed`。
  - thread 无 stage → `Completed`（`terminalReason = "no_stages_to_orchestrate"`）。
  - 存在非终态 stage 但都无可委派 agent → `Errored`（`lastErrorCode = "stage_without_assignable_agent"`，message 引导用户为 stage 配置 assistant）。
  - 存在非终态 stage 且都处于 `needs_review` → 按上文 `needs_review` 收敛规则处理。

### 内部 ACP backend

为 `pi_agent_rust` 增加独立内部 ACP backend 抽象，例如：

```text
InternalAcpBackendSessionSpec {
  purpose: Planning | Decision,
  command,
  workspacePath,
  model?,
  thinkingLevel?,
  timeoutMs,
  env,
  sessionDir?,
  metadata,
}
```

该抽象可以复用 `AcpTransport` 的协议实现，但不能复用普通 `StartAgentSession { agent: Agent }` 的公共语义。

必须满足：

- 不创建普通 chat session。
- 不写入 indexed historical agent session 表。
- 不触发普通 UI chat timeline。
- runtime events 必须带 `astraRunId`、`astraInternalPurpose`、`plannerBackend`，日志中与 delegated sessions 明确区分。
- session 结束后释放 controller、waiter、permission waiter 和 cancellation token。
- planner/decision session 的 transcript 若需要调试，只能作为 Astra run diagnostics 或 redacted log 保存，不进入 agent history。

## 状态、恢复与并发边界

- 每个 thread 同时最多一个 active Astra run；重复 start 必须返回现有 active run 或显式错误，不能创建第二个 active run。
- 每个 run 同时最多一个 orchestrator worker；进程内用 run-level lock / worker registry 防重入。
- 重复 start 命中 active run 时，必须做僵尸检测（见 Orchestrator「运行期僵尸 run 检测」）：active 但无 worker 的 run 标记 `interrupted` 后才允许接管。
- app 启动时必须处理 active runs：
  - 如果没有可恢复 worker，则标记为 `interrupted`，并记录 terminal/interruption reason。
  - 不自动恢复 delegated tasks；恢复必须是后续显式能力。
  - 已存在 delegated sessions 只做 best-effort cleanup，不取消无 Astra metadata 的用户会话。
  - **partial 占位 session 清理（v2 新增）**：delegated session linking 会写入 `partial: true` 的占位 session 记录，正常路径在拿到真实 agent session（含真实 jsonl 文件）后会被 promote。但 interrupted run 留下的占位记录永远不会被 promote，会一直挂在 stage/thread 上。recover 流程必须对 interrupted run 关联的、仍为 `partial` 且无真实文件的占位 session 做清理或标记不可用，避免 UI 残留幽灵 session。
- cancel 必须覆盖 planning、decision、dispatch waiting、running 四个状态：
  - planning/decision 中：取消内部 ACP session。
  - dispatch waiting 中：取消对应 delegated turn 并释放 waiter。
  - running 中：只取消该 run 创建的 delegated sessions。
  - cancelled 后 late event 必须被忽略或只记录诊断，不能重新推进 run。
  - **cancel 与阻塞 worker 的过程态（v2 新增）**：cancel 当前先 abort delegated sessions（drop waiter 会让阻塞中的 worker `recv` 返回 `Disconnected` 进而尝试 `fail_run`），再 `update_status(Cancelled)`。最终态始终是 `Cancelled`（`fail_run` 用 `update_active_status`，不覆盖已 terminal 的 run），但中间窗口 worker 可能在 run 仍 active 时抢先 emit 一个 `error` 事件，导致前端出现 error→cancelled 抖动。要求：先把 run 置为 `Cancelled`（占位 terminal），再 abort delegated sessions，使 worker 醒来时 run 已是 terminal，从而不 emit 多余 error；或让 worker 在 `Disconnected` 时先判定 run 是否正在被 cancel，再决定是否 emit。
- timeout 必须分层（**目标态**，按 Phase 落地）：
  - planner timeout —— 随 Phase 2 Pi planner 接入才需要（deterministic planner 同步执行，无需 timeout）。
  - decision timeout —— 随 Phase 2 Pi decision engine 接入才需要。
  - delegated task timeout —— **Phase 1 现状**：单一 delegated task timeout（当前硬编码 1 小时）。应改为可配置。
  - whole-run timeout 或 round limit —— **Phase 1 现状**：由 round limit 兜底（当前 `RUST_NATIVE_ROUND_LIMIT` 硬编码常量），应改为可配置 round limit，必要时叠加 whole-run wall-clock timeout。
- terminal 状态只允许一次写入；`completed`、`cancelled`、`errored`、`interrupted` 互斥。

## 权限、安全与配置边界

- `pi_agent_rust` planner/decision backend 默认不能请求项目文件写入权限，也不能 dispatch 外部 agent。
- 如果 ACP backend 发起 permission request，Astra 必须按 internal purpose 处理：
  - 默认 deny 未声明工具。
  - planning/decision 只允许读取型能力。
  - 任意写入型或 shell 型请求必须失败，并触发 deterministic fallback 或 run error。
- API key、provider config、base URL、env injection 必须 redaction 后进入日志和 diagnostics。
- **过渡期 apiKey 明文风险（v2 标注）**：legacy sidecar 路径会把 `provider.api_key` 明文放入 `astra/start` 的 params，经 stdin 传给 sidecar 子进程。在 sidecar 仍存在的 Phase 1-3 期间，这条传递链是明文的——必须确保 Rust 与 sidecar 两侧都不把 params/modelConfig 写入日志，并在 Phase 4 删除 sidecar 后彻底消除该链路。该风险仅限 `mode == "legacy_sidecar"` 的 run；rust-native 路径不经过 sidecar。
- planner command 必须来自配置白名单或用户明确配置；不能从 planner 输出反向决定 command。
- session dir/env injection 必须限定到 Astra internal session，不影响 delegated Codex/Claude/Gemini session。
- prompt 中的 memory/search 内容应有长度上限与敏感字段过滤；超长 snapshot 必须结构化裁剪，而不是简单无界拼接。
- deterministic fallback 不应掩盖安全拒绝：安全拒绝应记录为 policy failure；只有模型失败、非 JSON、超时、中断等可 fallback。

## 持久化与事件模型

为 `AstraRunRecord` 或其 JSON payload 补充以下字段。**这些字段在 v1 中已列出但代码尚未落地**；本版要求把 schema 扩展显式排进 Phase 1 与 Phase 2 之间（见「阶段实施」），避免 Phase 2 Pi 接入时再被迫改表：

- `plannerBackend`：`deterministic` / `pi_acp` / `sidecar_legacy`
- `decisionBackend`：`deterministic` / `pi_acp` / `sidecar_legacy`
- `roundIndex`
- `roundLimit`（取代硬编码 `RUST_NATIVE_ROUND_LIMIT`，支持配置）
- `terminalReason`（覆盖：`all_stages_terminal` / `no_stages_to_orchestrate` / `pending_human_review` / `no_more_work` / `round_limit_reached` / `cancelled` / `interrupted` 等）
- `activeWorkerId` 或 last worker marker（支撑运行期僵尸 run 检测）
- `internalPlannerSessionIds`
- `internalDecisionSessionIds`
- `lastErrorCode`（覆盖：`stage_without_assignable_agent` / `planner_no_dispatchable_tasks` / `worker_failed` 等）
- `lastErrorMessage`
- `runDiagnostics`（redacted）

事件 payload 统一包含：

- `runId`
- `threadId`
- `status`
- `eventType`
- `timestamp`
- `data`

事件类型至少包括：

- `status`
- `plan`
- `decision`
- `task_dispatch`
- `task_result`
- `stage_update_result`
- `issue_update_result`
- `retry_limit`
- `cancelled`
- `error`
- `completed`
- `interrupted`

所有事件都必须可由当前 run record 重放或解释；前端不能依赖仅存在于旧 sidecar protocol 的 transient 字段。

## 阶段实施

### Phase 1：领域收口与 Rust Orchestrator 雏形

目标：把 Astra 的业务边界从 sidecar 收回 Rust，但先不依赖 Pi，并补齐 deterministic planning + deterministic decision 的闭环。

> 说明：Phase 1 主体已落地（见「当前实现状态」）。本节其余条目区分为「已完成」与「v2 补强项」。

已完成：

- `src-tauri/src/astra/mod.rs` 已拆分出 `planner` / `decision` / `prompt` 模块（`types` / `orchestrator` 仍内联于 `mod.rs`，可后续继续拆分）。
- `AstraPlanner` / `AstraDecisionEngine` trait 与 deterministic 实现。
- orchestration loop 迁回 Rust：round limit、terminal stage 判定、dispatchable task 过滤、retry limit 处理、task result 驱动的 decision、decision 校验后的 stage/issue 更新。
- deterministic planning 逻辑迁回 Rust 作为默认 planner。
- deterministic decision policy：
  - completed result 且输出含明确完成信号 → stage `completed`。
  - completed 但含明确未完成信号 → stage `blocked`。
  - completed 无明确信号 → stage `needs_review`。
  - failed/errored result → 创建/更新 issue（thread-level 则 errored）。
  - cancelled result → cancel run。
  - retry limit reached → 组合「issue + blocked」决策。
- legacy sidecar 整条路径通过 `SESSIO_ASTRA_LEGACY_SIDECAR` 显式开关，默认 rust_native。
- run-level worker registry（`AstraWorkerGuard`）确保同一 run 只有一个 orchestrator worker。

v2 补强项（Phase 1 退出前必须完成）：

- **worker 顶层错误落地**：worker 闭包捕获 `Err`/panic 时必须把 run 落到 `Errored` 并 emit error，禁止只 log（见 Orchestrator「worker 异常终态保证」）。
- **运行期僵尸 run 检测**：`start_thread_astra` 命中 active run 时校验 worker 是否存活，僵尸 run 标 `interrupted` 后允许接管。
- **无 dispatchable tasks 的分类收敛**：区分 all-terminal / 空 stages / 无可委派 agent / 全 `needs_review` 四类终态，禁止统一 errored。
- **`needs_review` 收敛规则**：按「新领域契约」选定方案 A 或 B 落地，禁止 `needs_review` 撞 round limit 后误判 errored。
- **持久化 schema 扩展**：落地 `plannerBackend` / `decisionBackend` / `roundIndex` / `roundLimit` / `terminalReason` / `lastErrorCode` / `lastErrorMessage` 等字段，把 `RUST_NATIVE_ROUND_LIMIT` 与 retry limit 改为可配置。
- **interrupted run 的 partial 占位 session 清理**纳入 recover 流程。

交付标准：

- 新 orchestrator 可以在“不启用 Pi”的情况下完整跑完 planning -> dispatch -> decision -> mutation/complete。
- task dispatch、stage update、issue update、delegated session linking 与当前语义一致。
- delegated session linking 必须在 delegated task session start / agent session id 可持久化时完成，不能等 task terminal result 才 link。
- 旧 sidecar 路径仍可通过 feature flag 或 config 开关回退。
- app 重启后 active Rust-native runs 被标记 interrupted，不会 orphan；其 partial 占位 session 被清理。
- worker 任意提前返回 / panic 都使 run 落到 terminal，不留 active 僵尸。
- cancel 在 planning、decision、dispatch waiting、running 中都能释放 waiter 和 session，且不产生 error→cancelled 抖动。

退出标准：

- Rust deterministic orchestrator 在本地与测试环境可稳定运行。
- run 的创建、取消、完成、失败、重试上限都能被 Rust 直接管理。
- 不存在「status active 但 worker 已死」无法恢复的 run。
- 不再依赖 `astra/start` 才能推进默认 run。

### Phase 2：接入 `PiAcpPlanner`

目标：用 `pi_agent_rust` ACP 替代 sidecar 中的 TS Pi SDK planner 和结果决策能力。

实施内容：

- 新增内部 ACP backend 抽象，复用 ACP protocol/transport，但不创建普通 user agent session。
- 新增 `PiAcpPlanner`，通过内部 ACP backend 创建独立 planning session。
- 新增 `PiAcpDecisionEngine`，通过内部 ACP backend 创建独立 decision session。
- planning/decision session 使用短生命周期策略：
  - 每次 planning/decision 新建 session
  - 结果返回后立即结束/释放
  - 不复用 task execution session
- 将现有 Astra planning prompt 从 TS 迁到 Rust，并固定输出契约：
  - 仅返回 JSON object
  - shape 为 `{ summary, tasks[] }`
  - task 字段齐全且 agent/stage 必须可校验
- 增加 decision prompt，并固定输出契约：
  - 仅返回 JSON object
  - shape 为 `{ action, stage?, issue?, retry?, summary?, reason }`
  - action 必须属于 `update_stage` / `add_or_update_issue` / `retry_stage` / `plan_next_round` / `complete_run` / `error_run`
  - stage/issue 字段必须可由 Rust 校验
  - **`retry_stage` 与 `plan_next_round` 必须在 orchestrator 走不同控制流**（见「新领域契约」），并在持久化层记录区分。
- 在 Rust 中实现 planning result 处理：
  - 提取文本
  - JSON 修复/容错解析
  - 结构校验
  - sanitize 与 normalization
  - planner 失败时 fallback 到 deterministic planner
- 在 Rust 中实现 decision result 处理：
  - 提取文本
  - JSON 修复/容错解析
  - action 校验
  - stage/issue id 校验
  - retry limit guard
  - decision 失败时 fallback 到 deterministic decision engine
- 为 Pi planner 增加配置入口：
  - planner command / ACP command
  - model
  - thinking level
  - timeout（落地「目标态」中的 planner timeout）
  - session dir/env injection
- 为 Pi decision engine 增加同源配置入口（落地 decision timeout），可与 planner 共用 command/model，也可独立覆盖。
- 增加 redacted diagnostics，区分 planner failure、decision failure、policy denial、transport failure。

交付标准：

- 配置 `pi_agent_rust` 后，planning 与 decision 都可通过 ACP 完成。
- Pi 返回空结果、非 JSON、超时、取消、中断时，run 不崩溃，自动回退 deterministic planner/decision engine。
- planner 与 delegated task runtime 在日志和状态上能明确区分。
- internal ACP sessions 不出现在普通 chat UI 和 indexed historical sessions。
- 安全拒绝不会被误当作模型失败静默 fallback。
- `retry_stage` 与 `plan_next_round` 行为可在 diagnostics 中区分验证。

退出标准：

- 默认路径已经是 Rust orchestrator + Pi ACP planner/decision engine（当 Pi 已配置）。
- sidecar 只作为临时 fallback，不再承担主路径功能。

### Phase 3：替换前端/Tauri API 并移除 Astra 私有 RPC

目标：清理架构债，收敛成新的公开接口。

实施内容：

- 重新定义 Astra Tauri API，建议收敛为：
  - `create_astra_run`
  - `cancel_astra_run`
  - `list_astra_runs`
  - `get_astra_run`
- 重新定义事件流，建议收敛为单事件名，例如：
  - `astra-run-event`
- 前端统一改为新的 run/event 模型，不保留旧 Astra sidecar 协议痕迹。
- 迁移期间允许同一 PR 内短暂 adapter：
  - 新 API 与旧 API 可以同时存在于一个提交序列中。
  - 退出 Phase 3 前必须删除旧 command/event 引用。
  - 不发布长期双 API。
- 删除 Rust 中仅为 sidecar 服务的部分：
  - sidecar spawn
  - pending response map
  - `astra/start` / `astra/cancel` / `astra/task_result`
  - `tool/call` bridge
  - 私有协议 request/response/event 类型
  - sidecar disconnect recovery 逻辑

交付标准：

- 所有 Astra 操作只经过 Rust 内部 orchestrator。
- 前端不再依赖任何 sidecar 协议细节。
- Rust 中不存在 Astra 私有 RPC path。
- `src/api.ts` 与 UI hooks 只引用新 command/event。

退出标准：

- 从运行时角度，`sessio-astra` 已经不可达也不需要。
- 所有 run 都由 Rust 直接创建、推进、结束。

### Phase 4：删除 sidecar 与打包残留

目标：彻底删掉旧实现和分发残留。

实施内容：

- 删除 `sidecars/sessio-astra` 整个目录。
- 删除 Bun 相关构建脚本、smoke 脚本、lockfile。
- 删除 Tauri 配置中的 Astra sidecar binary 声明。
- 删除 capability 中对应的 sidecar command。
- 清理 README、开发文档、架构文档中关于 Astra sidecar 的描述。
- 更新测试和运维文档，只保留 Rust-native Astra 说明。
- 删除 legacy sidecar feature/config 开关（含 `SESSIO_ASTRA_LEGACY_SIDECAR`）。
- 删除 sidecar fallback 测试，只保留迁移前的历史说明或 changelog。

交付标准：

- 仓库、构建、打包、运行路径都不再包含 `sessio-astra`。
- 本地开发和发布产物不依赖 Bun sidecar。
- 过渡期的 apiKey 明文链路随 sidecar 一并消除。

退出标准：

- sidecar 完全删除后，应用功能不回退。
- 所有 CI / build / dev 文档与新架构一致。

## 迁移与兼容策略

- 采用双轨迁移：
  - Phase 1-2 期间保留旧 sidecar fallback。
  - fallback 通过显式 config/feature flag 开关，不自动隐式切换。
- 切换优先级：
  1. Rust orchestrator + Pi ACP planner/decision engine（当 Pi 配置完整且 policy 允许）
  2. Rust orchestrator + deterministic planner/decision engine（默认稳定 fallback）
  3. legacy sidecar fallback（仅迁移期，且必须显式开启）
- 当 Phase 3 完成时，关闭 fallback 并开始删除 sidecar 路径。
- 不保留长期双 API；一旦新前端 API 上线，旧命令和旧事件名直接清理。
- legacy sidecar fallback 是整条旧路径 fallback，不是 planner-only fallback；不能与 Rust-native orchestrator 混合写同一个 active run。
- 同一个 run 创建时必须固定 backend mode，run 中途不能从 Rust-native 切到 sidecar。

## 测试计划

必须覆盖以下场景：

### Planner 层

- deterministic planner 生成合法空/单任务/多任务计划
- deterministic planner 对「无 assistant 的非终态 stage」「空 stages thread」产出空计划（由 orchestrator 分类收敛）
- Pi planner 正常返回合法 JSON 计划
- Pi planner 返回非 JSON / 缺字段 / 非法 agent / 非法 targetStageId
- Pi planner 超时、取消、ACP 中断
- Pi planner fallback deterministic planner

### Decision 层

- deterministic decision 对 completed/failed/errored/cancelled result 生成保守合法 decision
- deterministic decision 对「completed 无明确信号」产出 `needs_review`，且 `needs_review` 收敛规则（方案 A/B）生效，不撞 round limit
- Pi decision 正常返回合法 JSON decision
- Pi decision 返回非 JSON / 非法 action / 非法 stage id / 非法 issue payload
- Pi decision 请求 retry 但 retry limit 已达时被 Rust 拒绝
- Pi decision 请求 mutation inactive run 时被 Rust 拒绝
- Pi decision 的 `retry_stage` 走「同一 task 立即重试」、`plan_next_round` 走「重新规划」，二者控制流可区分
- Pi decision 超时、取消、ACP 中断时 fallback deterministic decision

### Orchestrator 层

- run 启动后进入 planning -> dispatching -> running -> completed
- 所有 actionable stages 已终结时直接完成
- 空 stages thread 收敛为 completed（`no_stages_to_orchestrate`），不 errored
- 非终态 stage 全部无可委派 agent 时收敛为 errored 且 `lastErrorCode = stage_without_assignable_agent`
- retry limit reached 时 issue 更新正确
- cancel 在 planning 中、dispatch 中、running 中都能正确结束，且无 error→cancelled 抖动
- cancel 在 decision 中能正确结束
- round limit reached 时 run 正确标记 errored（`round_limit_reached`）
- worker 内部提前返回 Err / panic 时 run 落到 errored，不留 active 僵尸
- 同一 thread 重复 start 不创建第二个 active run
- 同一 run 不启动第二个 worker
- 重复 start 命中「active 但 worker 已死」的僵尸 run 时，标 interrupted 后允许接管
- late runtime event 不会推进 terminal run
- app 重启时 active run 被标记 interrupted，且其 partial 占位 session 被清理

### Runtime / delegation

- delegated task session 创建、session linking、snapshot 保存
- delegated session start 后、terminal result 前，UI/store 已能看到 session link
- task result 成功/失败/取消/错误四种路径
- planner session 与 delegated task session 不串状态
- decision session 与 delegated task session 不串状态
- internal ACP session 不出现在普通 chat session 列表
- internal ACP backend / delegated ACP agent 断开后 run 正确失败或回退
- permission request 在 planning/decision session 中按 policy allow/deny
- API key/env/config 日志脱敏（含 legacy sidecar params 不落日志）

### Frontend / API

- 新 command 与事件流能驱动现有 UI
- run 列表、详情、事件订阅、取消操作正常
- event payload 与 TS types 一致
- 不再引用旧 `thread-astra-event` 或 sidecar 特定字段
- 新旧 API adapter 在 Phase 3 结束前全部删除
- terminal/interrupted/error reason 能在 UI 中显示或至少可诊断

### Build / packaging

- 无 `sessio-astra` binary 时开发构建正常
- release bundle 不包含 Astra sidecar
- capability / tauri config 清理后运行无权限错误

## 里程碑与验收

- M1：Rust deterministic orchestrator 上线（含 v2 补强项：worker 终态保证、僵尸检测、边界分类收敛、needs_review 收敛、持久化 schema 扩展），旧 sidecar 可回退
- M2：`PiAcpPlanner` + `PiAcpDecisionEngine` 上线，并在 Pi 配置完整时作为默认智能 backend
- M3：前端切到新 Astra API / event model
- M4：sidecar、私有 RPC、打包残留全部删除

每个里程碑都必须满足：

- 集成测试通过
- 无 orphaned run / stuck run（含 worker 死亡、僵尸 active run）
- delegated session linking 不回退
- thread/stage mutation 语义保持稳定
- planner/decision/backend mode 可诊断（terminalReason / lastErrorCode 可解释每个终态）
- 安全、权限、日志脱敏检查通过

## 假设与默认决策

- 采用双轨迁移，而不是一次性切换。
- 允许重构前端/Tauri API，不要求保留旧命令和事件名。
- `pi_agent_rust` 仅作为 planner/decision backend，不承担 Astra orchestrator 语义。
- planner/decision session 采用短生命周期策略，不做长期复用。
- deterministic planner/decision engine 始终保留，作为 Pi backend 失败时的稳定 fallback。
- Astra run 的持久化与 stage/session linking 继续保留在 Rust store 层，不迁移到 agent 侧。
- `pi_agent_rust` 不加入普通 user agent enum，不进入 historical indexing。
- 自动恢复 interrupted run 不属于本迁移范围；本迁移只保证中断可见、可诊断、无 orphan。
- rust-native 路径默认全自动，不含人工审批；`AwaitingApproval` 仅为历史兼容保留，人工审批若需要须作为独立 Phase 显式实现。
- 所有 run 终态必须可由 `terminalReason` / `lastErrorCode` 解释；任何「卡在 active」的 run 都视为缺陷而非可接受状态。
