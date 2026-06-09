# Astra Runtime Session Lifecycle Plan

## 当前结论

Astra delegated task 在 v1 中采用 task-level、一次性的 ACP runtime session 策略：每个 plan task 新建一个 runtime session，用完即抛。

task 完成、报错、取消或超时后，都必须走同一条 cleanup 路径，释放 live runtime session。persisted agent session 继续作为 replay 和 UI 展示的数据来源，不承担 live runtime 复用职责。

这意味着 v1 的稳定关联点是 plan task 到 persisted session 的引用，而不是长期运行的 ACP 进程或 session。

一旦 delegated task 已经创建 live runtime session，后续任何失败都必须进入 finalize/cleanup 路径。失败点包括但不限于 placeholder link 失败、agent session id 尚未 ready、result record 失败、UI event emit 失败、等待 task waiter 失败。持久化或 UI 失败不能阻止 live runtime session 被释放，也不能让 batch/round waiter 永久挂起。

## 为什么不做进程/session 复用

当前 delegated task prompt 已经注入执行所需的 thread、stage、assistant、participant 和 plan snapshot。每个 task 都应被视为 self-contained job，不依赖历史 ACP session 的隐式上下文。

如果复用进程或 session，需要新增并维护更细粒度的映射：

- `planTaskId -> sessionId + turnId/laneKey`
- live message 的 turn slicing
- history replay 的 turn slicing
- lane-level terminal event 与 persisted transcript 的精确对齐

这会把当前的 task lifecycle 从 task-level reference 推到 turn-level source/link，复杂度和边界风险都明显上升。v1 先不做这个方向。

## Thread Kind 边界

不同 thread kind 的 runtime session 边界如下：

- `teamwork`: 每个 assistant task 独立 session。
- `process`: 每个 stage task 独立 session。
- `brainstorm`: 每个 participant task 独立 session。
- `debate`: 每个 participant/lane task 独立 session。
- `planner`: 独立 internal planner session/call。

`planner` 与 delegated runtime 分属不同生命周期。planner 可以为同一个 thread 生成 plan、round 或 batch，但不应和后续 participant/stage/assistant runtime session 共用 live session。

## Lifecycle

每个 delegated task 采用同一条生命周期：

1. dispatch task
2. link runtime placeholder
3. send input
4. receive terminal result
5. record result
6. emit UI event
7. dispose live runtime session
8. wake task waiter

完成、报错、取消和超时都必须进入 `dispose live runtime session`，然后唤醒等待该 batch 或 round 的协调逻辑。cleanup 不应依赖 UI 是否仍在当前页面，也不应依赖 replay 是否已经完成加载。

这里的 waiter 是 per task waiter，不是整个 batch 的单一 waiter。每个 task 终态后都必须唤醒自己的 waiter；batch/round 协调层再聚合这些 task result，并决定 round 是否继续、等待、失败或终止。

finalize 需要以 live runtime session id 为 cleanup key，并且必须是 best-effort、幂等的。即使 `record result` 或 `emit UI event` 失败，也要继续 dispose runtime session、移除 delegated runtime tracking、移除 waiter 并唤醒协调逻辑。Late runtime event 到达时不能重复写 task result，也不能重新打开已经释放的 live session。

`link runtime placeholder` 只表示 live session 已经创建，可用于 live UI、alias 和 cleanup；它不等同于 persisted agent session。agent runtime 上报真实 agent session id 后，应把 plan task 关联切到 persisted session，replay 优先使用 persisted `(agent, sessionId)`。如果任务终态前一直没有真实 agent session id，则该 task 应保留可诊断的缺失 replay 状态，而不是要求 history storage 支持 turn-level slicing。

## Failure Matrix

v1 需要把 lifecycle 每个步骤的失败分支写成显式 contract：

- `start_session` 失败：没有 live session 时不做 runtime cleanup，但必须把 task/run 记录为 dispatch/startup error，并把错误返回给 batch 协调层。不在同一次 attempt 内隐式重试；需要 retry 时必须生成新的 attempt/session，并记录 attempt id。
- `track_delegated_session`、`link runtime placeholder` 或 `task_waiter` 注册失败：如果 live session 已创建，必须立即 cancel/dispose，并唤醒 waiter 或向 batch 协调层返回 terminal error。
- `send_input` 失败：必须 cancel/dispose 当前 live session，移除 tracking/waiter，并把 task 记为 errored。
- `record_ready_delegated_session` 或 persisted session link/relink 失败：不得丢失 cleanup。task 可以继续用 live runtime id 完成，但必须记录 diagnostic，UI replay 进入 missing/partial 状态。
- `record result` 失败：仍要 dispose live runtime session、移除 tracking/waiter，并向 batch 协调层返回带 `result_persist_failed` diagnostic 的 terminal result。不能因为写库失败让 runtime 泄漏或 waiter 阻塞。
- `emit UI event` 失败：只记录日志/diagnostic，不影响 cleanup、waiter 和 task terminal state。
- `dispose` 失败：记录 `runtime_dispose_failed` diagnostic；cleanup 仍继续移除内存 tracking 和 waiter。dispose 需要有上层 timeout/force policy，不能无限等待。

cleanup 失败本身也必须有上限。优先走 graceful cancel，再 dispose；如果 ACP 子进程或 turn 无响应，需要 runtime manager 提供 bounded force-close/kill 能力，或者至少把 session 标记为 leaked 并从 Astra coordination 中剥离。

internal planner/orchestrator session 也必须遵守同样的 cleanup 原则。planner backend 如果在 subscribe、send prompt、等待 response 或解析 response 阶段失败，已经创建的 runtime session/ACP worker 也必须 bounded cleanup，不能因为它不是 delegated task 就跳过 dispose/force-close。

## Runtime Resource Gate

AstraPi 的限制目标是保护 CPU/内存，而不是保证同一个 thread 串行。因此不应把 v1 策略表述成 `per-thread single-flight gate`。per-thread single-flight 会和 `brainstorm` / `debate` 的并发 lane 需求冲突，也会让同一 batch 中的其他 runtime 被 AstraPi 串行化拖慢。

更清晰的策略是 resource limiter：

- planner limiter 和 runtime limiter 分开计数。
- runtime limiter 按 agent 或 runtime class 配额，例如 `astra-pi` 最多 N 个 live runtime；其他 agent 不应被 AstraPi 的限制串行化。
- limiter 是全局资源预算，可选叠加 per-thread fair queue，避免单个 thread 抢占全部 slot。
- queue 满时策略必须明确：排队、拒绝新 task，还是要求用户稍后重试；不杀旧 task。
- 即使 task 等待 limiter，拿到 slot 后仍然创建新的 runtime session，并在终态后释放。

等待 limiter 期间必须响应取消和终态变化。task 在入队前、拿到 slot 后、创建 runtime session 前都要确认 run/task 仍然 active；如果已经取消、超时或 run 已进入终态，不得再创建新的 live runtime session。

## Timeout Policy

timeout 需要分层，避免 limiter 等待、runtime 执行和 cleanup 互相污染：

- queue timeout: 从 task 进入 limiter queue 开始计算，只覆盖资源等待。超时后 task 变为 dispatch timeout，不创建 session。
- startup timeout: 从 `start_session` 开始计算，覆盖 runtime 启动和 session ready。
- execution timeout: 从 `send_input` 成功或 turn started 开始计算，覆盖 agent 执行。
- cleanup timeout: 从 cancel/dispose 开始计算，超时后进入 force-close/leaked diagnostic。
- whole-run timeout/round limit: 防止 planner/round 无限推进。

batch timeout 不应只用一个 wall-clock deadline 覆盖所有 task。parallel batch 应按 task attempt 计时；sequential round 应区分当前 running task 与尚未 dispatch 的 planned task。

## Cancellation Policy

用户取消 run/task 时：

- 已创建 live runtime session 的 task 先发 runtime cancel/cancel_turn；如果 bounded grace period 内未终态，再 dispose/force-close。
- 未创建 session、仍在 limiter queue 中的 task 直接从 queue 移除并标记 cancelled，不启动 runtime。
- 已产生 persisted agent session 的内容不回滚。UI replay 保留 partial/cancelled transcript，并通过 task status、terminal reason 和 diagnostic 解释它是不完整结果。
- cancellation finalize 必须和 timeout/error finalize 共用同一条幂等 cleanup 路径。

## Task Identity 与 Idempotency

v1 的稳定 id 应该是 plan task id。`AstraTaskProposal.id` 可以在 plan 生成前作为 transient id，但写入 `thread_plan_tasks` 后，dispatch/result/retry 都必须使用 `plan_task_id` 或把 `task.id` 规范化为 plan task id。不能依赖“两个字段刚好相等”的隐式约定。

每次 runtime dispatch 都需要 attempt identity：

- `attempt_id` 或 `(plan_task_id, attempt_count)` 用于 runtime metadata、UI event、diagnostic 和 waiter key。
- 同一个 attempt 的 terminal result、UI event、waiter wake 只能生效一次。
- retry 必须创建新的 live runtime session，并保留旧 attempt 的 terminal state/session ref；不能覆盖旧 persisted transcript。
- `thread_plan_task_sessions` 需要允许同一个 plan task 下多个 attempt/session 可解释地共存，或明确只保留 latest 并把旧 session 标为 superseded。

## Startup Recovery

app/process 重启后，live runtime session 无法可靠恢复。startup recovery 需要：

- 把 active Astra run 标为 interrupted。
- recovery、`get_active_astra_run` 和 worker active check 必须共用同一组 active statuses，至少覆盖 `planning`、`thinking`、`awaiting_approval`、`dispatching`、`running`。不能手写 SQL status list 后漏掉某个中间态。
- 把 running plan task 标为 interrupted/errored，或至少写入 diagnostic，避免 UI 永远显示 running。
- 清理只存在 placeholder、没有真实 persisted file 的 partial Astra session。
- 保留已有 persisted agent session，不做 destructive cleanup。
- replay 对缺失 session ref 显示 missing/partial，而不是阻塞页面加载。

## Replay 与 History

v1 replay 继续按 `(agent, sessionId)` 聚合 persisted session。

plan task 通过 `thread_plan_task_sessions.session_id` 关联 runtime 产生的 persisted session。UI 可以用 plan task、round、stage、assistant、participant 和 lane 信息决定展示位置，但不要求 history storage 支持 turn-level source/link。

live 到 persisted 是 eventually consistent。UI 先通过 live runtime event 和 runtime alias 显示 lane；当 `SessionStarted`/delegated ready event 带来真实 agent session id 后，再刷新 thread replay/history 并切到 persisted session。刷新前 replay 可能出现 missing/404，这必须是合法中间态。

ready event、task dispatch、task result、run terminal event 都应能触发相关 UI 状态刷新。UI 不应假设 persisted session 已经立即落盘；history loader 必须容忍 session ref 存在但 session file 尚不可用。

v1 不新增 turn-level schema。只有未来引入 process pool、长期 ACP session 复用，或同一个 persisted session 内承载多个 plan task 时，才需要先补充 turn-level source/link 与 live/history turn slicing。

## Verification Notes

推荐验证：

- `cargo test --manifest-path src-tauri/Cargo.toml astra::`
- Debate 两个 lane 独立显示。
- Brainstorm 每个 participant 独立 lane。
- Process stage task 可按 stage/plan task 回放。
- Teamwork assistant task 可按 round/task 回放。
- Brainstorm/Debate 在 resource limiter 下仍能按预期并发显示 lane，不被 per-thread single-flight 串行化。
- AstraPi 连续多轮后不会残留多个高 CPU live runtime，且全局 runtime 资源预算生效。
- placeholder link 失败、record result 失败、UI event emit 失败时仍释放 live runtime session 并唤醒 task waiter。
- AstraPi runtime task 等待 limiter 期间取消后，不再创建新的 live runtime session。
- agent session id 未 ready 就终态时，task 有可诊断状态，replay/UI 不挂起。
- start_session 失败、send_input 失败、dispose 超时、persisted session 保存失败都有可验证的 task/run diagnostic。
- internal planner/orchestrator backend 失败时也会释放已创建 runtime session，不残留 planner ACP worker。
- task retry 不会重复写 terminal result、重复 emit terminal UI event、重复 wake waiter。
- app 重启后 active run/task 不会停留在 running，placeholder-only session 被清理或标为 missing。

## Current Implementation Gaps

当前代码还没有完全满足本 plan 的 lifecycle contract。以下项在实现前应视为 blocking gaps：

- `finish_delegated_task` 中 `record_task_result` 或 `emit task_result` 失败会提前返回，导致 live runtime session release 和 task waiter wake 被跳过。这里必须改成 best-effort finalize：持久化/emit 失败只写 diagnostic，不能阻止 cleanup 和 waiter wake。
- AstraPi delegated runtime 仍使用 per-thread single-flight lock，key 形如 `runtime:{threadId}:{threadKind}`。这会让同一个 brainstorm/debate thread 内多个 AstraPi participant/lane 串行，也可能拖慢同 batch 的非 AstraPi task。应替换为 planner/runtime 分离的 resource limiter。
- delegated task timeout 仍是单一 batch wall-clock deadline，尚未拆成 queue、startup、execution、cleanup timeout。limiter 等待时间不应挤占 runtime execution timeout。
- runtime cleanup 尚无 bounded force-close/kill policy。当前 cancel/dispose 可以从 Astra coordination 中剥离 session，但还没有明确证明 ACP worker/子进程会在 cleanup timeout 内终止。
- failure diagnostic 尚未覆盖 `start_session`、`send_input`、`dispose`、persisted session link/relink、persisted result 保存失败等路径。
- startup recovery 应覆盖所有 active statuses，包括 `thinking`，并收敛 running plan task 与 placeholder-only sessions。当前 recovery 只中断 active run，不足以保证 plan task/session placeholder 一起闭合。
- retry/attempt identity 尚未持久化；`thread_plan_task_sessions` 只有 `(task_id, agent, session_id, role)`，需要明确多 attempt session 的解释方式，或扩展 schema 保存 attempt identity。
- planner/orchestrator session 的 cleanup 还不具备与 delegated runtime 等价的 bounded cleanup contract。planner per-thread 串行本身可以保留，但它不能替代 planner/runtime 分离的 resource limiter。

## Future Work

如果后续确实需要进程池或长期 session 复用，应先设计 turn-level link/source：

- 明确 `planTaskId -> sessionId + turnId/laneKey` 的持久化位置。
- live message 按 turn/lane 切片。
- history replay 按 turn/lane 切片。
- terminal result 与 task/lane 的关联必须可验证。

在这些基础能力完成前，delegated runtime 继续保持一次性 session 策略。
