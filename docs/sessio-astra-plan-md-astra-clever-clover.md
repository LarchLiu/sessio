# 修订 docs/sessio-astra-plan.md 以支撑「闭环编排」

## Context（为什么要改）

用户要的是一个**闭环**：Astra 依据 thread/stage 委派任务 → 接收任务结果 → 据结果形成 stage 更新决策 → 告知 Sessio 更新 stage 状态 → Sessio 执行更新并把成功/失败通知 Astra → Astra 据最新 stage 状态派发下一个任务 → 循环直到 thread 最后一个 stage 完成。

当前 `docs/sessio-astra-plan.md` 只覆盖闭环的**前半段**，是一次性「提案 → 审批 → 派发 → monitor」的单轮模型。纯文档层面对照用户 6 项要求，存在 3 个缺口：

- **缺口 A（接收任务结果）**：协议是「Astra 驱动、Rust 应答」的单向控制流，没有任何「被委派的 ACP 任务完成、结果为 X」回推给 Astra 的消息；工具表里也没有等待/查询任务结果的工具。第 13 步的「monitor」只是断言，无机制。
- **缺口 B（再派发 + 闭环终止）**：User Flow（318–331）单轮结束于 monitor，没有「任务完成后再唤醒 Astra 评估并派发下一步」的循环，run 状态（active/errored/interrupted）也没有「thread 全部 stage 完成 → completed」的终止条件。
- **缺口 C（审批冲突）**：359 行「dispatch_task 在用户审批对应 task id 前必须失败」+ 单轮提案，与「自动派发直至最后 stage」互斥。

**已与用户确认的两项决策（本次修订据此落地）：**
1. **审批模型 = 一次审批后自动跑**：用户一次性批准整份多 stage 计划，之后 Astra 自动循环，人工介入最少。
2. **stage 更新方式 = Astra 判断后向 Sessio 提交更新决策，Sessio 执行更新并回传结果**：Astra 是决策者，不是执行者；落地执行者是 Sessio Rust service/store。Astra 只能通过受控协议表达「建议/请求把某 stage 更新为某状态、summary、outcome 或 issue」，Sessio 负责校验 thread/stage 归属、权限、运行状态和数据约束，更新完成后再以工具响应或事件通知 Astra 成功/失败。
3. **重试同一 stage + Sessio 侧阈值熔断**（用户追加）：Astra 对结果不满意时可对**同一 stage** 重新委任任务；**由 Sessio（非 Astra）记录每个 stage 的派发次数并设可配置阈值**，超阈值时 Sessio 拒绝再直接派发并通知 Astra，由 Astra 重新决策。计数放 Sessio 侧以防 LLM 无限重试烧 token/ACP 资源。

本计划的**交付物是对设计文档 `docs/sessio-astra-plan.md` 的修订**（markdown 编辑），不涉及代码实现。

---

## 待确认的依赖（修订时需在文档「Assumptions」标注）

- 决策 2 不应依赖 GUI 调用 `sessio` CLI，也不应让 Astra shell 出去执行 CLI。修订文档时按「Astra 提交 stage 更新决策，Sessio service 执行并回传结果」来写。CLI 可以继续作为外部 agent/skill 的稳定接口，但不是 Astra 内部编排的执行通道。
- Astra 委派 stage 任务产生的 runtime session 应由 Sessio 挂到对应 thread stage 的 `sessions` 下；只有不属于具体 stage 的 thread 级任务才挂到 thread 顶层 session 列表。

---

## 修订方案（逐节改 docs/sessio-astra-plan.md）

### 1. Tools 节（295–316 行）：补上闭环的执行语义

- `sessio.agent.dispatch_task` 改为**阻塞式**：派发后等待被委派的 ACP turn 到达终态（completed / failed / cancelled / errored），其工具响应**返回结构化任务结果** `AstraTaskResult`（见第 3 节）。这是 Astra「接收到任务结果」的主通道——Astra 调用即等待返回，天然衔接「根据返回的结果做判断」。
  - 标注权衡：ACP 任务可能长时间运行或中途触发权限请求；阻塞期间权限交互走现有 Sessio 权限 UI，turn 结束后该工具调用才 resolve。需配超时与取消联动（取消 run 即取消其在途派发）。
  - 备选机制（文档中作为 alternative 记一笔，V1 不采用）：非阻塞 `dispatch_task` + 新增 Rust→Astra 的 `task-completed` 通知消息，由 sidecar 重新唤醒 Astra agent。
- `sessio.stage.update` 与 `sessio.stage.issue.add_or_update` 改述为**Astra 的决策提交接口，而非 Astra 的执行工具**：Astra 根据任务结果提交期望的 stage 状态/summary/outcome 或 issue 变更；Sessio 在 Rust service/store 内执行校验和写入；写入成功返回更新后的 stage/issue，失败返回结构化错误。明确「执行者是 Sessio，Astra 只是决策者」。这满足要求 3（更新 stage）与要求 4（接收更新成功/失败），同时避免把 CLI 当成 GUI/sidecar 内部执行层。

### 2. 用 "Orchestration Loop" 重写 User Flow（318–331 行）

把单轮流程改写成**两阶段 + 自动循环**：

- **提案阶段**：Astra 读 `sessio.project.snapshot`/`sessio.memory.search`，对每个待办 stage 调 `sessio.agent.plan_task` 记录任务，emit 一次完整多 stage 的 plan 事件。
- **一次审批**：用户批准整份计划，前端调 `confirm_thread_astra(runId, approvedTaskIds[])`；确认后该 run 进入 auto 执行态。
- **自动执行循环**（sidecar 内单个长生命周期 Astra agent run 驱动）：
  1. `dispatch_task(下一个 stage 的任务)` → 阻塞等待 → 得到 `AstraTaskResult`
  2. Astra 判断结果 → 向 Sessio 提交 `sessio.stage.update` 决策请求 → Sessio 校验并写回 stage 状态/summary/outcome → Astra 收到成功/失败结果
  3. 结果不满意 / 失败 / 有 blocker → 可对**同一 stage** 再次 `dispatch_task` 重试；每次派发都由 Sessio 累加该 stage 的 attempt 计数。同时调 `sessio.stage.issue.add_or_update` 记 blocker
  3b. 当某 stage 的 attempt 达到 Sessio 配置阈值 → Sessio **熔断**：拒绝对该 stage 再直接派发，并通过 dispatch 返回 `retryLimitReached`（或专门信号）**通知 Astra 重新决策**——换 agent / 换 prompt / 拆分任务 / 标记 stage blocked / 终止 run，而非沿同一路径死磕
  4. 重新 `snapshot` 评估 stage 状态 → 选下一个任务 → 回到 1
  5. **终止条件**：snapshot 显示 thread 最后一个 stage 到达终态（全部 stage done）→ emit `complete` 事件 → run 标记 `completed`
- 说明：阻塞式 dispatch 让整个循环自洽在**一个 Astra agent run** 内（每个工具结果驱动下一步决策），无需外部重新唤醒——这是闭环驱动的关键。补一句上下文增长的考量（多 stage 后可重新 snapshot 收敛上下文）。

### 3. 协议 / 数据类型（193–293 行）：新增任务结果、stage 决策结果与 run 模式

- 在「Rust Data Types」（196–208 行）新增 `AstraTaskResult`，字段建议：`sessioRuntimeSessionId`、`stageId`、`status`(completed/failed/cancelled/errored)、`summary`/`finalMessage`、`error?`、`attemptCount`（该 stage 已派发次数）、`retryLimitReached`（是否已触发熔断）。
- 新增 `AstraStageDecision` / `AstraStageMutationResult`（命名可实现时调整）：
  - `AstraStageDecision` 表示 Astra 的决策意图：`threadStageId`、期望 `status`、`summary?`、`outcome?`、`issue?`、依据的 `taskResultId/sessionId`、决策理由。
  - `AstraStageMutationResult` 表示 Sessio 的执行结果：`ok`、更新后的 `stage`/`issue`、`error?`、`appliedAt`。它由 Sessio 生成，Astra 不能伪造。
- `AstraRun`（210–222 行）新增：`mode`（auto）、当前循环位置（如 `currentStageId` / 已完成 task ids）、**按 stageId 的 attempt 计数 map**、**重试阈值配置**（可配置，默认建议 3），用于 UI 展示进度、熔断判断与重启恢复。
- 「Rust To Astra JSONL」节：dispatch_task 的 Response `result` 示例填成 `AstraTaskResult` 形状；stage 更新的 Response/Event 示例填成 `AstraStageMutationResult`。若采用备选异步机制，再补一个 `method: "task-completed"` / `method: "stage-update-result"` 的事件样例（标注为 alternative）。

### 4. Permission Boundary（343–350 行）：放宽到 run 级确认

- 把「`sessio.agent.dispatch_task` 必须在对应 task id 被用户审批前失败」改为：**在 run 被 `confirm_thread_astra` 确认前失败；确认后，Astra 可在该 run 内自动派发本 thread 计划内的任务，无需逐任务再确认**。
- 保留：取消 run 作为「急停」；取消只影响本 run 派发的会话；ACP agent 自身的权限请求仍走现有 Sessio 权限 UI。
- 明确 auto 模式下 Astra 自动派发的边界（仅限 thread 内已规划 stage 的任务）。
- **阈值熔断为 Sessio 侧强制规则**（与 State Authority 一致）：每次 `dispatch_task` 由 Rust 累加对应 stage 的 attempt 计数；达到阈值后 Rust 拒绝对该 stage 再直接派发并回传熔断信号，防止 Astra（LLM）无限重试烧 token/ACP 资源。计数与阈值由 Sessio 维护，Astra 不能自行绕过。

### 5. Run 生命周期 / Recovery（374–390 行）：加 completed 终态与恢复位点

- run 状态枚举新增 `completed`；定义 thread 完成 = 所有 stage 达终态（最后一个 stage done）。
- 持久化的 run metadata 增加循环位点（currentStageId / 已完成 task ids）与**按 stageId 的 attempt 计数**，重启后 active→`interrupted` 时 UI 能显示停在哪一步、各 stage 已重试几次（V1 不做自动续跑，仅可见可手动接管）。

### 6. Implementation Steps / Test Plan / Assumptions

- **Implementation Steps（392–407 行）**：第 9 步「approved task dispatch」扩成「阻塞式 dispatch + 返回 AstraTaskResult」；新增「自动执行循环与终止条件」「Astra stage 决策请求由 Sessio service/store 落地并回传结果」「持久化循环位点」步骤。
- **Test Plan（409–445 行）**新增用例：
  - 阻塞式 dispatch 在 turn 终态返回结果；超时与取消能正确解除阻塞。
  - 全计划审批后，循环按 stage 顺序自动推进，无需逐任务再确认。
  - `sessio.stage.update` 决策请求由 Sessio 执行，成功/失败都被 Astra 正确接收并据此决策。
  - 终止条件：最后一个 stage done 后 run 置 `completed` 并 emit `complete`。
  - 结果不满意 → 对同一 stage 重试，Sessio 的 attempt 计数正确递增。
  - attempt 达阈值 → Sessio 熔断拒绝再派发、回传 `retryLimitReached`，Astra 走重新决策分支（换 agent / 标记 blocked / 终止），不卡死循环。
  - 任务失败 → 记 issue，不会无限重试同一路径。
- **Assumptions（447–457 行）**：新增「auto 模式（一次审批后自动跑）」「Astra 不直接执行 `sessio` CLI；内部编排通过 Sessio Rust service/tool bridge 落地状态变更」「同一 stage 重试次数由 Sessio 按 run 内 stageId 计数、阈值可配置（默认建议 3）、超阈值熔断并通知 Astra 重新决策」三条。

---

## 验证（doc 改完后）

文档自检（沿用文档现有 Document Verification 风格）：

```bash
rg -n "Orchestration Loop|AstraTaskResult|AstraStageMutationResult|completed|dispatch_task" docs/sessio-astra-plan.md
git status --short docs/sessio-astra-plan.md
```

逐项核对 6 要求在改后文档中均有明确机制（不再是断言）：
1. 依据 thread/stage 委派 → snapshot + dispatch_task（原已覆盖）
2. 接收任务结果 → 阻塞式 dispatch_task 返回 AstraTaskResult（新增）
3. 更新 stage → Astra 提交 stage 决策请求，Sessio 执行更新（改写）
4. 接收更新成功/失败 → Sessio 工具 response/event（明确）
5. 据 stage 状态再派发 → Orchestration Loop 执行阶段（新增）
6. 闭环直至最后 stage → completed 终态 + 终止条件（新增）
7. 重试同一 stage + 超阈值通知重新决策 → Sessio 侧按 stageId 的 attempt 计数 + 可配置阈值熔断（新增）

> 注：本次仅修订设计文档。代码层（dispatch 阻塞或异步结果回传实现、RuntimeManager 的 turn 终态钩子、Sessio service/store 的 stage mutation result）需在实现阶段另行核验。`sessio` CLI 仍可作为外部 agent/skill 的接口，但不作为 Astra 内部编排的执行者。
