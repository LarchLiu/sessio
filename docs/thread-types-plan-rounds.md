# Thread 类型与 Plan Round 记录方案

## 摘要

本文档定义 Sessio thread 的新类型体系，以及所有 thread 类型共享的 plan round / task 记录模型。

目标是把 thread 从单一的“项目下工作项”扩展为几种协作模式：

- `workflow`：人为定制 stages，分阶段顺序执行，无需 Astra 自动编排。
- `teamwork`：人为配置 assistants，由 Astra agent 按 task-centric 流程编排执行。
- `brainstorm`：多个不同模型共享上下文，各抒己见，再逐轮汇总发散。
- `debate`：PK mode 的正式命名，两个或多个模型上下文隔离，交叉验证直到收敛。

同时，每一轮 plan 都需要被持久化。plan round 记录本轮 tasks，并明确多 task 是 `parallel` 还是 `sequential`。UI reload、Astra 诊断、历史回看都应能从 plan/task 状态恢复，而不是依赖瞬时事件或单个 `currentTaskId`。

thread 还必须是全过程容器。一个 thread 下直接对话、stage task、plan task、Astra 内部 plan/synthesis/diagnostic 产生的所有 sessions，都需要能从 thread 反查并重放。

plan task 还必须保存执行时的 stage / assistant / agent 配置快照。`thread_stage_id`、`assistant_id`、`target_agent` 只负责关联和导航，不能作为历史重放的唯一事实源，因为同一个 stage、assistant 或 agent 后续都可能被重新配置。

## 背景问题

### 1. thread 当前没有类型

当前 `threads` 表主要包含 `project_id`、`goal`、`description`、`stage_id`、`enabled` 等字段。现有 `workflow` 概念属于 project/stage 模板体系，而不是 thread 自身的协作模式。

这导致所有 thread 在产品语义上都被当成同一类对象处理。实际上用户需要几种不同的工作方式：

- 人为定制 stages，并按顺序推进的 workflow。
- 人为配置 assistants，交给 Astra 拆 task、调度和执行的 teamwork。
- 多模型共享上下文、并行发散再汇总的 brainstorm。
- 多模型上下文隔离、交叉验证、直到统一的 debate。

### 2. assistants 只绑定在 workflow/stage 层，不适合所有 thread 类型

`workflow` 类型可以继续通过 stage assistants 分配任务。但 `teamwork`、`brainstorm`、`debate` 不一定需要 stages，它们更自然的模型是 thread-level assistants。

因此需要为 thread 本身增加 assistants 绑定关系，作为非 workflow 类型的协作成员列表。

### 3. 每轮 plan 还不是一等对象

此前设计曾把 Astra task lifecycle 放进 `astra_runs` 的 `proposed_tasks_json` / `task_results_json` 一类字段里，但这不是“每轮 plan”的清晰建模，也不再作为新实现的存储结构：

- 不能自然表达第几轮 plan。
- 不能明确一轮中的 tasks 是并行还是串行。
- reload 后 task running 状态恢复依赖其他字段或 live event。
- 普通 thread 类型也没有 plan/task 历史记录能力。

后续需要把 plan round 和 plan task 从 Astra 内部字段提升为 thread 级别的一等数据。旧 Astra run lifecycle 字段和旧 stage-decision 逻辑直接删除；因为这批 schema 还没有 release，不需要为旧 `astra_runs` 表形状做兼容迁移。

### 4. thread 与 sessions 的关系还不够完整

当前 thread 可以有直接 `thread_sessions`，workflow stage 也可以有 `stage_sessions`。但如果后续引入 plan round/task、teamwork、brainstorm、debate，只靠直接挂在 thread 上的 sessions 不足以重放全过程。

需要明确：thread 是所有相关 sessions 的根容器。任何由 thread 触发的 session，即使它归档在 stage、plan task、Astra internal diagnostic 或 debate lane 下，也必须能通过 thread 查询出来。

### 5. task 只保存 id 无法还原当时的执行配置

plan task 如果只保存 `thread_stage_id`、`assistant_id`、`target_agent`，历史重放时会读到这些对象的当前配置，而不是 task 执行时的配置。

典型问题：

- stage 的 prompt、assistant 分配或排序后来被改过。
- assistant 的 system prompt、model、tools、runtime agent 后来被改过。
- agent 的 backend、model 参数或执行策略后来被改过。

因此 task 需要同时保存引用和快照：引用用于跳转到当前对象，快照用于审计、重放、结果解释和 issue 回看。

## Thread 类型定义

### `workflow`

`workflow` 是带 stages 的阶段式工作流。

它可以理解为当前产品里 deterministic 工作模式的正式 thread kind：流程由人预先定义，系统按确定的 stage 顺序记录和推进。注意这里的 deterministic 是产品语义，不等同于代码里的 `DeterministicOrchestratorBackend` fallback；workflow 不由 Astra 自动调度。

适用场景：

- 软件开发：plan -> code -> review。
- 写作：research -> outline -> draft -> edit。
- 视频生产：script -> storyboard -> production -> review。

行为约定：

- 继续使用现有 `thread_stages`。
- task 可以绑定 `thread_stage_id`。
- stage 主要作为路由、上下文和 session 归档。
- stage 顺序由人定义，默认按阶段顺序执行。
- 无 Astra scheduling：workflow 的下一步由用户、UI 或显式人工规则推进，不由 Astra planner 决定。
- workflow thread 可以同时拥有 thread-level assistants，但 v1 不要求使用。

### `teamwork`

`teamwork` 是带 thread-level assistants 的团队协作模式。

适用场景：

- 多个 assistants 按能力拆分同一个目标。
- 一个 assistant 调研，一个 assistant 实现，一个 assistant 检查。
- 不需要固定 stage 顺序，但需要明确成员和任务分工。

行为约定：

- 不要求 stages。
- tasks 主要绑定 thread-level assistant 或 target agent。
- plan mode 可以是 `parallel` 或 `sequential`。
- 适合持续多轮推进。
- 复用 `docs/astra-task-centric-refactor-plan.md` 的 task-centric 编排模型。
- `docs/astra-task-centric-refactor-plan.md` 是 teamwork 的编排标准；它不是 workflow 的 Astra 调度方案，而是把现有 Astra task routing 从 stages 迁移为 thread-level assistants。
- Astra task-centric 的标准路由对象是 `assistantId` 或 thread-level assistant。
- teamwork 不读取 stage status，也不需要 stage/issue mutation。

### `brainstorm`

`brainstorm` 是带 thread-level assistants 的发散模式。

适用场景：

- 多 assistants 各自提出方案。
- 多视角生成选题、产品方向、技术路线。
- 希望先发散，再由用户或 Astra 汇总。

行为约定：

- 不要求 stages。
- 默认 plan mode 倾向 `parallel`。
- 多个 assistants 可以同时产出不同想法。
- 后续 round 可以进入汇总、筛选或扩展。
- 所有参与模型共享 thread 初始上下文。
- 每一轮结束后，Astra 生成 shared board，保留观点、亮点、冲突点和待展开问题。
- 下一轮所有模型都可以看到 shared board，再继续补充、反驳、延展。
- 最终由 Astra synthesis 输出候选方案、共识、分歧和推荐。

### `debate`

`debate` 是 PK mode 的正式命名。

适用场景：

- 多个 assistants 持不同立场辩论。
- 比较多个方案的优劣。
- 先分别陈述，再互评，最后汇总结论。

行为约定：

- 不要求 stages。
- 需要 thread-level assistants。
- v1 中每轮 plan 只能整体标记为 `parallel` 或 `sequential`。
- “先并行产出观点，再串行交叉质询，再汇总”的复杂流程，用多轮 plan 表达，而不是在一轮内建 DAG。
- 每个参与模型运行在 isolated lane 中，彼此不共享完整上下文。
- 第一轮各 lane 只看到同一份初始问题。
- 交叉验证时，lane A 只能看到 lane B 的阶段性产物，不看 B 的完整对话过程；lane B 同理。
- Astra 比较各 lane 结论，若一致则输出统一答案；若不一致则生成下一轮交叉验证 task。
- 达到 round limit 仍不一致时，输出共识部分、分歧部分和 Astra 裁决建议。

命名说明：

- 选择 `debate` 而不是 `pk`，是因为 `debate` 更适合产品语义和英文 API。
- UI 可以显示为“PK / Debate”，但存储值使用 `debate`。

## 工作模式目标与编排实现

### Workflow: 人为流程控制

workflow 的目标是让用户可以手工定制阶段和顺序。系统记录阶段、任务和结果，但不让 Astra 决定下一步。

执行方式：

1. 用户配置 stages 和每个 stage 的 assistants。
2. 用户或 UI 按顺序启动某个 stage 的工作。
3. stage task 可写入 plan round/task，便于统一历史回看。
4. stage 完成后由用户推进到下一 stage。

实现重点：

- 保留现有 workflow/project/stage 能力。
- plan round 在 workflow 中主要作为记录和 replay 层，不作为调度状态机。
- workflow task 优先绑定 `thread_stage_id`。

### Teamwork: Assistant 路由的 Astra Task-Centric 编排

teamwork 的目标是让用户只配置团队成员，Astra 负责拆解、分派和推进。

它以 `docs/astra-task-centric-refactor-plan.md` 为标准编排模型：

```text
assistant = 路由 / 上下文 / agent 配置 / session 归档
task      = 执行事实 / 生命周期状态 / 结果记录
run       = 编排进度 / cursor / 终态
```

这个迁移不是把 workflow 变成自动调度器，而是把当前 Astra stage-routed task-centric 流程改成 assistant-routed task-centric 流程：旧实现里的 stage route、stage status 和 stage issue mutation 被 `assistant_id`、plan task lifecycle 和 task result 取代。

执行方式：

1. 用户选择 thread-level assistants。
2. Astra 根据 thread goal、assistant system prompts 和历史 task results 生成下一轮 tasks。
3. 每个 task 绑定 `assistant_id`，并可派发给 assistant 对应的 runtime agent。
4. parallel round 中多个 assistants 可同时工作。
5. sequential round 中按 `sort_order` 一个个执行。
6. task results 回写 plan task 状态，Astra 再决定下一轮 tasks 或 run 终态。

实现重点：

- 不需要 stages。
- 不读取 stage status。
- 不让 LLM 返回 stage/issue mutation。
- running/planned/completed 从 plan task 状态恢复。
- prompt contract 与 Astra task-centric refactor 保持一致，task shape 使用 `assistantId`，可同时保留 `targetAgent`。

### Brainstorm: 共享上下文的多模型发散

brainstorm 的目标是让多个不同模型在共享上下文下各抒己见，逐轮发散和汇总。

执行方式：

1. Round 1 使用 `parallel`，所有 assistants 看到相同 thread context 和用户目标。
2. 每个 assistant 独立输出观点、方案或问题清单。
3. Astra 读取本轮所有结果，生成 shared board。
4. 下一轮 assistants 都看到 shared board，并继续补充、反驳、扩展。
5. 需要结束时，Astra 生成 synthesis：候选方案、共识、分歧、推荐。

实现重点：

- `parallel` / `sequential` 只描述本轮 task dispatch 方式，不等于 brainstorm 的完整编排逻辑。
- Astra orchestrator 需要新增 brainstorm 专用编排能力，例如 `brainstorm_backend` 或等价策略模块，负责 shared board 的生成、持久化、下一轮注入和最终 synthesis。
- 需要在 plan round 或 diagnostics 中记录 shared board。
- 同一轮内 task 并行，不互相等待。
- 下一轮 prompt 显式包含上一轮 shared board。
- brainstorming 的 task 不应要求收敛过早；前几轮以广度和差异为优先。

### Debate: 隔离上下文的交叉验证

debate 的目标是让两个或多个模型彼此独立思考，再交叉验证，直到结论统一或明确分歧。

执行方式：

1. Round 1 使用 `parallel`，每个 lane 只看到同一份初始问题。
2. 各 lane 产出自己的答案、依据和置信度。
3. Astra 只交换阶段性产物，不交换完整对话上下文。
4. Cross-check round 中，lane A 审查 lane B 的产物，lane B 审查 lane A 的产物。
5. Astra 比较修正后的答案：
   - 如果一致，生成统一结论。
   - 如果不一致，生成下一轮交叉验证 task。
   - 如果达到 round limit，输出共识、分歧和裁决建议。

实现重点：

- `parallel` / `sequential` 只描述某一轮的 dispatch 方式，不足以表达 debate 的 lane 隔离、artifact 可见范围和一致性判断。
- Astra orchestrator 需要新增 debate 专用编排能力，例如 `debate_backend` 或等价策略模块，负责 lane lifecycle、cross-check task 生成、阶段性产物交换、convergence 判断和 round limit 处理。
- 需要给 plan task 或 plan round 增加 lane 概念，至少能区分 A/B。
- lane context 必须隔离；不能把一个 lane 的完整 transcript 直接喂给另一个 lane。
- cross-check 只能传递对方阶段性产物和 Astra 摘要。
- debate 的终态可以是统一，也可以是保留分歧。

## Plan Round 模型

### 核心目标

所有 thread 类型都记录每轮 plan。

每轮 plan 必须回答：

- 这是第几轮。
- 本轮为什么这样安排。
- 本轮 tasks 是并行还是串行。
- 每个 task 分配给哪个 stage / assistant / agent。
- 每个 task 执行时使用的 stage / assistant / agent 配置快照是什么。
- 每个 task 当前状态是什么。
- 每个 task 产出了什么结果或错误。

### 并行与串行

每轮 plan 有一个 `mode`：

- `parallel`：本轮 tasks 可以同时 dispatch。
- `sequential`：本轮 tasks 按 `sort_order` 一个接一个 dispatch。

v1 不支持一轮内混合 DAG，例如“前两个并行，完成后第三个汇总”。这类流程用多轮 plan 表达：

```text
Round 1: parallel  -> 多 assistant 各自产出
Round 2: sequential 或 single task -> 汇总/评审
```

### Plan Task 状态

plan task 至少支持：

- `planned`
- `running`
- `completed`
- `failed`
- `errored`
- `cancelled`

UI reload 后必须从 task 状态恢复展示，而不是依赖 live event 或单个 `currentTaskId`。

## Thread Sessions 与全过程重放

thread 必须是全过程容器。无论是哪种 thread 类型，所有与该 thread 相关的 sessions 都需要能被关联、查询和重放。

### Session 关联范围

一个 thread 的可重放过程包含：

- 直接挂在 thread 上的 sessions。
- workflow stage 下的 `stage_sessions`。
- plan task dispatch 产生的 delegated sessions。
- Astra internal planning / synthesis / decision sessions 的可诊断引用。
- brainstorm 的 shared-board / synthesis 生成 session。
- debate 的 isolated lane 和 cross-check sessions。

现有 `thread_sessions` / `stage_sessions` 可以继续保留。新增 plan task 后，task session 也必须能回连到 thread。也就是说，thread 的 replay 不应只看 `ThreadInfo.sessions` 或 `thread_sessions`，而应该聚合：

```text
thread direct sessions
+ stage sessions
+ plan task sessions
+ Astra internal diagnostic session refs
```

聚合时按 `(agent, session_id)` 去重，并保留来源：

- `thread`
- `stage`
- `plan_task`
- `astra_internal`

### Replay 时间线

thread replay 应按统一时间线展示全过程：

1. thread 创建和用户输入。
2. plan round 创建。
3. plan tasks planned/running/terminal。
4. delegated session 过程和结果。
5. stage/session 归档变化。
6. Astra synthesis / shared board / debate convergence。
7. run 或 round 终态。

排序优先级：

- plan round 的 `round_index`。
- plan task 的 `sort_order`。
- session / task / event timestamps。

workflow replay 可以按 stage 分组；teamwork/brainstorm/debate replay 可以按 round 分组；debate 还需要按 lane 分组。

### 存储原则

session 本体仍由现有 `sessions` 表保存。thread/plan 只保存引用，不复制 transcript。

plan task 的 session 引用必须包含 agent 和 session id，因为 session 的稳定身份是 `(agent, session_id)`，单独 `session_id` 不足以唯一定位。

一个 plan task 可能关联多个 session，例如 delegated runtime session、agent runtime session、synthesis session 或 cross-check session。因此 plan task 不应只保存单个 `session_id` 字段，而应使用 task-session 关联表。

写入原则：

- 直接打开或继续的普通会话写入 `thread_sessions`。
- workflow stage 产生的会话写入 `stage_sessions`，并可通过 `thread_stages.thread_id` 回连 thread。
- plan task 派发产生的会话写入 `thread_plan_task_sessions`，并可通过 `thread_plan_rounds.thread_id` 回连 thread。
- Astra 内部 planner / synthesis / diagnostic session 至少保留在 run diagnostics 中；进入 replay API 时按 `(agent, session_id)` 合并展示。

## 数据模型建议

### `threads.kind`

在 `threads` 表新增：

```sql
kind TEXT NOT NULL DEFAULT 'workflow'
```

允许值：

```text
workflow | teamwork | brainstorm | debate
```

旧 thread 数据迁移后默认读取为 `workflow`。这只是保证历史数据可读，不表示旧 Astra stage-decision 自动调度路径需要继续兼容。

### `thread_assistants`

新增 thread-level assistants 绑定表：

```sql
thread_assistants(
  thread_id TEXT NOT NULL,
  assistant_id TEXT NOT NULL,
  sort_order INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(thread_id, assistant_id),
  FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
  FOREIGN KEY(assistant_id) REFERENCES assistants(id) ON DELETE RESTRICT
)
```

用途：

- `teamwork` 的团队成员。
- `brainstorm` 的发散参与者。
- `debate` 的辩论参与者。

`workflow` 可以保留为空，继续优先使用 stage assistants。

### `thread_plan_rounds`

新增 plan round 表：

```sql
thread_plan_rounds(
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  astra_run_id TEXT,
  round_index INTEGER NOT NULL,
  summary TEXT,
  mode TEXT NOT NULL,
  source TEXT NOT NULL,
  status TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(thread_id, round_index),
  CHECK(mode IN ('parallel', 'sequential')),
  CHECK(source IN ('astra', 'manual', 'agent')),
  CHECK(status IN ('planned', 'running', 'completed', 'cancelled', 'errored')),
  FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
  FOREIGN KEY(astra_run_id) REFERENCES astra_runs(run_id) ON DELETE SET NULL
)
```

建议索引：

```sql
CREATE INDEX IF NOT EXISTS idx_thread_plan_rounds_thread_index
  ON thread_plan_rounds(thread_id, round_index);

CREATE INDEX IF NOT EXISTS idx_thread_plan_rounds_thread_status
  ON thread_plan_rounds(thread_id, status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_thread_plan_rounds_astra_run
  ON thread_plan_rounds(astra_run_id);
```

字段说明：

- `astra_run_id`：如果本轮由 Astra 生成，则关联 Astra run；普通人工/agent plan 可为空。
- `round_index`：thread 内从 0 或 1 递增，具体实现可按现有 Astra round 习惯选择，但必须稳定；`UNIQUE(thread_id, round_index)` 防止 reload 或并发 planner 写入重复轮次。
- `mode`：本轮 task 执行策略。
- `source`：计划来源。
- `status`：round 级状态，可由 tasks 聚合更新。
- 后续 brainstorm/debate 可在 round 层扩展 `shared_board_json`、`synthesis_json`、`convergence_status` 等诊断字段；v1 可先放入 diagnostics 或 summary，不阻塞基础表落地。

### `thread_plan_tasks`

新增 plan task 表：

```sql
thread_plan_tasks(
  id TEXT PRIMARY KEY,
  round_id TEXT NOT NULL,
  thread_stage_id TEXT,
  assistant_id TEXT,
  target_agent TEXT NOT NULL,
  stage_snapshot_json TEXT,
  assistant_snapshot_json TEXT,
  agent_snapshot_json TEXT NOT NULL,
  title TEXT NOT NULL,
  prompt TEXT NOT NULL,
  expected_output TEXT,
  risk TEXT NOT NULL,
  sort_order INTEGER NOT NULL,
  status TEXT NOT NULL,
  result_summary TEXT,
  error TEXT,
  started_at INTEGER,
  completed_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  CHECK(risk IN ('low', 'medium', 'high')),
  CHECK(status IN ('planned', 'running', 'completed', 'failed', 'errored', 'cancelled')),
  FOREIGN KEY(round_id) REFERENCES thread_plan_rounds(id) ON DELETE CASCADE,
  FOREIGN KEY(thread_stage_id) REFERENCES thread_stages(id) ON DELETE SET NULL,
  FOREIGN KEY(assistant_id) REFERENCES assistants(id) ON DELETE SET NULL
)
```

字段说明：

- `thread_stage_id`：workflow task 可绑定 stage。
- `assistant_id`：teamwork/brainstorm/debate task 可绑定 thread assistant。
- `target_agent`：实际 runtime agent。
- `stage_snapshot_json`：task 创建或 dispatch 时的 stage 快照；无 stage task 可为空。
- `assistant_snapshot_json`：task 创建或 dispatch 时的 assistant 快照；无 assistant task 可为空。
- `agent_snapshot_json`：task 创建或 dispatch 时的 runtime agent 快照。
- `sort_order`：串行 round 的执行顺序，也用于 UI 稳定排序。
- 后续 debate 可在 task 层扩展 `lane_id` 和 `visible_artifact_ids_json`，用于表达隔离 lane 和交叉验证可见范围；v1 可先通过 metadata/diagnostics 记录。

建议索引：

```sql
CREATE INDEX IF NOT EXISTS idx_thread_plan_tasks_round_order
  ON thread_plan_tasks(round_id, sort_order);

CREATE INDEX IF NOT EXISTS idx_thread_plan_tasks_round_status
  ON thread_plan_tasks(round_id, status, sort_order);

CREATE INDEX IF NOT EXISTS idx_thread_plan_tasks_stage
  ON thread_plan_tasks(thread_stage_id);

CREATE INDEX IF NOT EXISTS idx_thread_plan_tasks_assistant
  ON thread_plan_tasks(assistant_id);
```

快照建议至少包含：

- stage：`id`、`title/name`、`kind`、`instructions/prompt`、`sort_order`、当时绑定的 assistant ids。
- assistant：`id`、`name`、`role`、`system_prompt`、`model/provider`、`tools`、`runtime_agent`、关键执行参数。
- agent：`agent`、`backend`、`model`、`temperature`、`timeout_ms`、`tools/capabilities`、关键环境约束。

快照字段应保存 task 执行所需和事后解释所需的信息，但不复制完整 transcript。后续对象被重新配置时，历史 task 的快照不回写、不重算。

### `thread_plan_task_sessions`

新增 plan task session 关联表：

```sql
thread_plan_task_sessions(
  task_id TEXT NOT NULL,
  agent TEXT NOT NULL,
  session_id TEXT NOT NULL,
  role TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(task_id, agent, session_id, role),
  CHECK(role IN ('primary', 'delegated', 'runtime', 'planner', 'synthesis', 'cross_check', 'diagnostic')),
  FOREIGN KEY(task_id) REFERENCES thread_plan_tasks(id) ON DELETE CASCADE
)
```

字段说明：

- `(agent, session_id)`：session 的稳定身份。
- `task_id`：session 来源 task，可通过 task -> round -> thread 回连 thread。
- `role`：session 在 task 中的用途；同一个 session 可以在 replay 聚合时按 `(agent, session_id)` 去重。

用途：

- 多并行 task reload 后恢复每个 task 对应的 running session。
- thread replay 聚合 plan task sessions。
- debate cross-check 记录某个 lane 的审查 session。
- brainstorm synthesis 记录汇总 session。

## 行为说明

### 创建 thread

`create_thread` 支持：

- `kind`
- `assistantIds`

默认：

- `kind = workflow`
- `assistantIds = []`

如果 `kind != workflow`，UI 应鼓励选择至少一个 assistant，但后端 v1 可以只做宽松校验，避免阻塞数据迁移和测试。

### 更新 thread

`update_thread` 支持修改：

- goal
- description
- enabled
- kind
- assistantIds

如果从 `workflow` 切换到非 workflow：

- 不删除已有 stages。
- stages 保留为历史和可回退结构。
- 非 workflow 的新 plan 默认使用 thread assistants。

如果从非 workflow 切换到 `workflow`：

- 不删除 thread assistants。
- workflow 仍优先使用 stage assistants。

### Plan round 创建

每次 Astra 或人工生成计划时：

1. 创建 `thread_plan_rounds`。
2. 创建对应 `thread_plan_tasks`。
3. 为每个 task 固定 stage / assistant / agent 执行快照。
4. 根据 `mode` 在事务中 dispatch：
   - `parallel`：所有 planned tasks 同时进入 running。
   - `sequential`：只启动 sort_order 最小的 planned task。
5. task terminal 后更新 task status。
6. sequential round 中，前一个 task terminal 后在同一事务中启动下一个 planned task。
7. 所有 tasks terminal 后更新 round status。

执行不变量：

- 创建 round 时必须用事务同时写入 round 和全部 tasks；如果任一 task 写入失败，不能留下空 round。
- `round_index` 必须在 thread 内唯一；Astra 创建下一轮时应在同一事务中分配并写入，避免两个 planner 并发生成同一个 round。
- `parallel` round 可以同时有多个 `running` tasks。
- `sequential` round 在任一时刻最多只能有一个 `running` task。启动下一个 task 时必须在同一事务中检查同 round 不存在其他 `running` task，并只选择 sort_order 最小的 `planned` task。
- sequential task terminal 和 next task running 的更新应作为一个原子状态转换；reload 不应看到“前一个 terminal 已写入，但下一个仍未启动”的长期中间态，除非 run 已被 cancel/error 打断。
- round status 由 task status 聚合：存在 running 则 round running；全部 terminal 后按结果聚合为 completed/cancelled/errored。

快照固定时机：

- manual/agent plan：创建 task 时保存快照。
- Astra plan：planner 输出 task 并落库时保存快照；如果 dispatch 前配置发生变化，不隐式刷新快照，除非显式 replan。
- retry/rework：创建新的 task 或新的 round，并保存新的快照；不修改旧 task 快照。

### 与 Astra 的关系

Astra run 只保留编排元数据、backend、diagnostics、round cursor 和终态原因；task lifecycle 必须直接使用 `thread_plan_rounds` / `thread_plan_tasks` / `thread_plan_task_sessions`。旧 `proposedTasks` / `taskResults` / `currentTaskId` / `approvedTaskIds` 存储字段、API 展示字段和 orchestrator 逻辑都删除，不做只读归档兼容，也不从旧 stage-decision contract 反推新的 lifecycle。

目标行为：

- Astra 每轮 planner 输出 tasks 后，先写 plan round/task。
- dispatch 从 plan task 读取，并使用 task 内的 stage/assistant/agent 快照构造 prompt 和 runtime 参数。
- task result 回写 plan task status 和 result summary。
- UI 从 plan tasks 展示 planned/running/completed。
- teamwork 是 Astra task-centric 流程的 assistant-routed 版本：没有 stages，Astra 根据 assistants 生成和派发 tasks。
- brainstorm/debate 在通用 plan round/task 之上增加不同的上下文策略：brainstorm 共享 shared board，debate 使用 isolated lanes。

### 与 workflow stages 的关系

`workflow` thread 的 plan task 可以绑定 `thread_stage_id`。

stage 仍然负责：

- 路由。
- 上下文。
- stage assistant 配置。
- session 归档。

stage 不负责表达 task lifecycle。task 状态由 `thread_plan_tasks.status` 表达。

workflow task 的 `thread_stage_id` 用于导航和分组，执行和 replay 使用 `stage_snapshot_json`。如果 stage 后续被重命名、改 prompt 或换 assistant，旧 task 仍显示和使用当时的 stage 快照。

### 与 assistants 的关系

非 workflow thread 的 task 主要绑定 `assistant_id`。

如果 task 只指定 `target_agent` 而没有 assistant：

- 允许保存。
- UI 展示为 agent-level task。
- 后续可以补充 assistant binding。

如果 task 同时有 `assistant_id` 和 `target_agent`：

- `assistant_id` 表示产品成员。
- `target_agent` 表示实际 runtime agent。
- 二者通常一致，但不强制完全一致，以支持 assistant 配置变化后的历史回看。

assistant 和 agent 的历史解释以 `assistant_snapshot_json` / `agent_snapshot_json` 为准。`assistant_id` 和 `target_agent` 用于关联当前对象、过滤和分组，不用于覆盖历史执行配置。

## 分阶段执行方案

推荐实施顺序：

1. 先实现 `ThreadKind` / `thread_assistants`，让 thread 可以表达四种协作模式并绑定 thread-level assistants。
2. 直接实现 `thread_plan_rounds` / `thread_plan_tasks` / `thread_plan_task_sessions`，建立所有 thread kind 共享的 plan/task 事实源。
3. 改 Astra 新 contract，让 planner 输出 `{ summary, runIntent, reason, mode, tasks }`，并把 plan round/tasks/session refs 写入上述表。
4. 接入 teamwork：使用 `docs/astra-task-centric-refactor-plan.md` 的 assistant-routed task-centric 编排，不要求 stages，不读 stage status，不写 stage/issue mutation。
5. 最后再做 brainstorm / debate；它们分别需要 shared-board backend 和 isolated-lane / cross-check backend，不能只靠普通 teamwork planner 宣称完成。

### Phase 1: 新增 thread kind 和 thread assistants

目标：先让 thread 类型和 thread-level assistants 可持久化。

执行内容：

- Rust 新增 `ThreadKind` enum。
- `ThreadInfo` 新增 `kind` 和 `assistants`。
- `threads` 表新增 `kind`，旧数据默认 `workflow`。
- 新增 `thread_assistants` 表。
- store trait / sqlite / cached / Tauri command / TS API 支持创建和更新 thread kind + assistant ids。
- ProjectPage 创建/编辑 thread 时可选择类型和 assistants。

验收：

- 旧 thread 读取为 `workflow`。
- 新建四种 thread 类型后 reload 仍正确。
- teamwork/brainstorm/debate 可绑定 thread assistants。
- workflow 现有 stage 行为不变。

### Phase 2: 新增 plan rounds/tasks 持久化

目标：建立每轮 plan 和 task 的事实源。

执行内容：

- 新增 `PlanRoundInfo`、`PlanTaskInfo` 类型。
- 新增 `thread_plan_rounds` / `thread_plan_tasks` 表。
- `thread_plan_tasks` 保存 stage / assistant / agent 执行快照字段。
- 新增 `thread_plan_task_sessions` 表，保存 plan task 到 `(agent, session_id)` 的引用。
- 新增 list/get/create/update task status 的 store API。
- 新增 link/list task sessions 的 store API，用于 task dispatch 后记录 delegated/runtime/synthesis/cross-check sessions。
- `ThreadInfo` 可选择包含最近 plan rounds，或通过单独 command 查询。
- TS API 增加对应类型和 wrapper。

验收：

- 可以创建 parallel round 和 sequential round。
- 可以更新 task status。
- 一个 task 可以关联多个 sessions，且每个引用都包含 agent 和 session id。
- 一个 task 保存当时的 stage / assistant / agent 快照，后续配置变化不影响旧 task。
- reload 后 round/tasks 状态不丢。
- 删除 thread 时 plan rounds/tasks/task sessions cascade 删除。

### Phase 3: Astra 写入 plan rounds/tasks

目标：Astra 每轮 plan 都写入 plan round/tasks。

执行内容：

- Astra planner 输出 tasks 后创建 `thread_plan_rounds`。
- 将每个 `AstraTaskProposal` 映射为 `thread_plan_tasks`。
- 写入 task 时解析并保存当时的 stage / assistant / agent 快照。
- dispatch 时更新 plan task 为 running。
- dispatch/result 到达时写入 `thread_plan_task_sessions`，session 身份使用 `(agent, session_id)`。
- result 到达时更新 plan task terminal 状态、result summary、error。
- `AstraHandle` 只暴露 run 元数据、backend、diagnostics 和终态信息；task 展示从 plan round/task 查询，或由服务端基于 plan tables 显式派生，不能从 run lifecycle 字段读取。

验收：

- Astra 每轮 plan 在 DB 中有 round 记录。
- 多 task 并行时所有 task running 可恢复。
- 每个 delegated session 都能从对应 plan task 反查。
- dispatch 使用 task 快照，而不是读取 stage/assistant/agent 的最新配置覆盖旧 task。
- sequential mode 能按顺序 dispatch。
- Astra terminal 后 round status 正确聚合。

### Phase 4: Teamwork 接入 assistant-routed task-centric 编排

目标：把 `docs/astra-task-centric-refactor-plan.md` 定义的 assistant-routed teamwork 编排接入产品。

执行内容：

- Teamwork prompt 使用 thread goal、thread assistants、assistant system prompts、历史 plan task results。
- Teamwork task shape 使用 `assistantId`，可同时保留 `targetAgent`。
- Astra planner 只返回 tasks 和 run intent，不返回 stage/issue mutation。
- Dispatch 时按 `assistant_id` 找到 assistant 的 runtime agent 和 system prompt。
- Task result 回写 `thread_plan_tasks`，并驱动下一轮 plan。

验收：

- Teamwork thread 不需要 stages 也能由 Astra 自动拆解和执行。
- 多 assistants parallel task 可同时运行并在 reload 后恢复。
- Sequential teamwork round 按 `sort_order` 执行。
- Teamwork 不读取 stage status，不写 stage/issue mutation。

### Phase 5: Brainstorm 接入 shared-board 编排

目标：实现共享上下文的多模型发散模式。

执行内容：

- 新增 brainstorm 专用 orchestrator 策略，例如 `brainstorm_backend`，不能只复用普通 task-centric planner。
- `brainstorm_backend` 负责在每轮 task terminal 后读取本轮结果，生成 shared board，并把 shared board 固定到 plan round diagnostics/summary 或后续结构化字段。
- `brainstorm_backend` 负责构造下一轮 task prompt，显式注入最近 shared board 和必要的历史 board 摘要。
- `brainstorm_backend` 负责判断是否继续发散、进入扩展 round，或生成最终 synthesis round。
- Brainstorm 默认第一轮使用 `parallel`，每个 assistant 获取相同 thread context。
- Astra 在每轮结束后生成 shared board，记录观点、亮点、冲突点、待展开问题。
- 下一轮 prompt 给所有 assistants 注入 shared board。
- 最终 synthesis round 输出候选方案、共识、分歧、推荐。
- Shared board v1 可写入 plan round summary/diagnostics，后续再独立结构化字段。
- 如果 v1 不实现 brainstorm 专用 orchestrator，则 Phase 5 只能标记为 schema/API 准备，真正 shared-board 编排推迟到 v2，不能宣称已实现 brainstorm mode。

验收：

- Brainstorm 多 assistants 能并行产出不同观点。
- 每轮 task terminal 后确实生成 shared board，并且 reload 后可追踪。
- 第二轮 assistants 能看到上一轮 shared board。
- 下一轮 task prompt 中能验证 shared board 被注入，而不是只依赖隐式历史上下文。
- Synthesis 输出能区分共识和分歧。
- Brainstorm 不使用 isolated lane；所有参与者共享汇总上下文。

### Phase 6: Debate 接入 isolated-lane 交叉验证

目标：实现 PK mode：隔离上下文、交叉验证、直到统一或明确分歧。

执行内容：

- 新增 debate 专用 orchestrator 策略，例如 `debate_backend`，不能只复用普通 task-centric planner。
- `debate_backend` 负责为每个 assistant 创建和维护 isolated lane，并记录 lane id、lane artifact 和可见 artifact 范围。
- `debate_backend` 负责生成 cross-check tasks：只向某个 lane 暴露对方阶段性产物或 Astra 摘要，不暴露完整 transcript。
- `debate_backend` 负责比较各 lane 的最新结论，判断 converged / diverged / need_more_cross_check / round_limit_reached。
- Debate 为每个 assistant 创建 lane，lane 之间不共享完整 transcript。
- Round 1 各 lane 只看到同一份初始问题。
- Cross-check round 只交换对方阶段性产物或 Astra 摘要，不交换完整上下文。
- Astra 比较 lane 输出，若一致则 complete；不一致则生成下一轮 cross-check tasks。
- Round limit 到达仍不一致时，输出共识、分歧和裁决建议。
- `lane_id` 和可见 artifact 范围 v1 可先放入 task metadata/diagnostics，后续结构化。
- 如果 v1 不实现 debate 专用 orchestrator，则 Phase 6 只能标记为 schema/API 准备，真正 isolated-lane 编排推迟到 v2，不能宣称已实现 debate mode。

验收：

- Debate lane A/B 的完整上下文互不泄漏。
- Cross-check task 只能看到对方阶段性产物。
- 每个 lane 的可见 artifact 范围可审计，reload 后仍能解释某个 task 当时看到了什么。
- Convergence 判断由 debate backend 明确记录，包括一致、不一致、继续交叉验证或达到 round limit 的理由。
- 一致时输出统一结论。
- 不一致且达到 round limit 时输出共识和分歧。

### Phase 7: 前端展示和 reload 恢复

目标：让用户能看到 thread 类型、assistants 和 plan history。

执行内容：

- 新增 thread replay 查询/API，聚合 `thread_sessions`、`stage_sessions`、`thread_plan_task_sessions` 和 Astra diagnostic session refs。
- Thread card 展示 kind。
- 非 workflow thread 展示 assistants。
- Thread detail / Astra panel 展示 plan rounds。
- task card 状态从 `thread_plan_tasks.status` 恢复。
- task detail 展示当时执行快照，并可跳转到当前 stage / assistant / agent。
- parallel/sequential 用清晰标签展示。
- Replay 视图按 thread kind 分组：workflow 按 stage，teamwork/brainstorm 按 round，debate 按 round + lane。
- Replay 聚合结果按 `(agent, session_id)` 去重，同时保留来源标签。

验收：

- 切换界面再回来，running tasks 仍正确。
- 用户能看到每轮 plan 的 summary、mode、tasks。
- 用户能从 thread 入口看到所有相关 sessions，并打开对应 transcript。
- 用户能看到 task 执行时的 stage / assistant / agent 配置，即使当前配置已变化。
- workflow task 显示绑定 stage。
- teamwork/brainstorm/debate task 显示绑定 assistant。
- Brainstorm 展示 shared board / synthesis。
- Debate 展示 lane、交叉验证和最终收敛状态。

### Phase 8: 删除旧 run lifecycle 存储和逻辑依赖

目标：避免长期两套事实源冲突。旧 Astra run lifecycle 字段来自未发布实现，直接移除，不做 archive compatibility。

执行内容：

- 从 Rust run record、SQLite DDL、Tauri API、TS API、UI 和 orchestrator 中删除 `proposedTasks` / `taskResults` / `currentTaskId` / `approvedTaskIds` 等 run lifecycle 字段。
- 新 UI 和新 orchestrator 逻辑只以 plan rounds/tasks 为准。
- 删除依赖 `currentTaskId` / `approvedTaskIds` 恢复 running 的逻辑。
- 新 Astra 自动编排不接受旧 stage-decision contract，也不通过旧字段恢复或继续调度。
- SQLite 当前版本直接修改 schema；旧表形状未 release，不写兼容迁移。
- 更新测试，确保 plan tasks 是 lifecycle owner。

验收：

- running/planned/completed 展示不依赖 `currentTaskId`。
- 多并行 task reload 稳定。
- `astra_runs` 不包含旧 lifecycle 列。
- `AstraHandle` 不包含旧 lifecycle 字段。

## 测试矩阵

### Migration

- 旧 DB 中 threads 自动获得 `kind = workflow`。
- 新表创建幂等。
- 删除 thread cascade 删除 thread assistants、plan rounds、plan tasks、plan task sessions。

### Thread 类型

- 创建/读取/更新 `workflow`。
- 创建/读取/更新 `teamwork`。
- 创建/读取/更新 `brainstorm`。
- 创建/读取/更新 `debate`。
- 非 workflow thread 可以绑定多个 assistants。

### Plan Round

- 创建 parallel round，多个 tasks 可同时 running。
- 创建 sequential round，只按 sort_order 启动 task。
- task terminal 后 round status 聚合正确。
- reload 后 task status 和 round status 保持一致。
- task session refs 可保存多个 `(agent, session_id, role)`。
- task 创建后保存 stage/assistant/agent 快照。
- stage/assistant/agent 后续改名、改 prompt、改 model 后，旧 task replay 仍显示旧快照。

### 绑定关系

- workflow plan task 可绑定 `thread_stage_id`。
- teamwork/brainstorm/debate plan task 可绑定 `assistant_id`。
- task 无 assistant 但有 target agent 时可保存和展示。
- task 的 id 关联用于当前对象跳转，历史执行解释使用 snapshot。

### Astra

- 每轮 Astra plan 写入 round/tasks。
- 多 task parallel round reload 后全部 running。
- sequential round 按顺序 dispatch。
- task result 更新对应 plan task。
- task dispatch/result 写入 `thread_plan_task_sessions`，而不是只写单个 session id。
- Astra dispatch 使用 `thread_plan_tasks` 中的 stage/assistant/agent 快照。

### Thread Replay

- thread replay 聚合直接 `thread_sessions`。
- thread replay 聚合 workflow stage 下的 `stage_sessions`。
- thread replay 聚合 plan task 下的 `thread_plan_task_sessions`。
- thread replay 聚合 Astra planner/synthesis/diagnostic session refs。
- replay 结果按 `(agent, session_id)` 去重，不按裸 `session_id` 去重。
- workflow replay 可按 stage 分组。
- teamwork/brainstorm replay 可按 round 分组。
- debate replay 可按 round + lane 分组。
- 同一个 session 同时来自 thread/stage/task 时只展示一次，但保留多个来源标签。
- replay 展示 task 当时的 stage/assistant/agent 快照，不被当前配置覆盖。

## 风险与取舍

### 风险 1: thread kind 与 project workflow 命名容易混淆

`workflow` 既是 project/stage 模板概念，也是 thread kind。实现时要在代码命名中区分：

- `WorkflowInfo`：项目/阶段模板。
- `ThreadKind::Workflow`：thread 协作模式。

同时，`ThreadKind::Workflow` 可以被理解为当前 deterministic 产品流程，但不能直接等同于代码里的 `DeterministicOrchestratorBackend` fallback。后续实现应把 deterministic backend 当作过渡或测试能力，而不是 workflow thread 的产品定义。

UI 文案可以解释为“Workflow thread”或“阶段式工作流”。

### 风险 2: 一轮只能 parallel 或 sequential

v1 不支持复杂 DAG，这是有意取舍。复杂流程通过多轮 plan 表达，降低实现复杂度。

### 风险 3: 旧 run lifecycle 双轨风险

Astra 旧字段 `proposedTasks/taskResults/currentTaskId/approvedTaskIds` 如果继续存在，会让 UI、orchestrator 和 reload 恢复出现双轨事实源。处理方式不是保留只读归档，而是直接从 schema、API、UI 和逻辑层删除；新 UI 和新 orchestrator 必须以 plan tasks 为唯一 lifecycle owner。

### 风险 4: non-workflow thread 是否允许无 assistants

产品上非 workflow thread 最好至少有一个 assistant。但 v1 后端建议宽松允许，UI 做提示，避免迁移和测试复杂化。

### 风险 5: session replay 来源过多导致重复或遗漏

thread replay 会同时聚合 thread、stage、plan task 和 Astra diagnostic 来源。实现时必须用 `(agent, session_id)` 作为稳定身份，并保留 source labels，否则容易出现同一 session 重复展示，或 plan task session 无法从 thread 找回。

### 风险 6: 只保存 id 导致历史执行配置漂移

stage、assistant、agent 都是可配置对象。历史 task 如果 replay 时读取当前配置，会把后来修改的 prompt、model、tools 或 timeout 套到旧执行上，导致审计和 debug 失真。实现时必须把 id 关联和执行快照分开：id 用于导航和聚合，snapshot 用于执行、重放和解释。

### 风险 7: Brainstorm / Debate 复杂度被 round mode 低估

`parallel` / `sequential` 只解决 task dispatch 维度，不能表达 shared board 生成、下一轮注入、lane isolation、artifact 可见范围、cross-check 和 convergence 判断。

因此 Phase 5/6 必须二选一：

- 实现专用 orchestrator 策略：`brainstorm_backend` 负责 shared-board 编排，`debate_backend` 负责 isolated-lane / cross-check 编排。
- 或者明确 Phase 5/6 只是 schema/API 准备，把真正的 brainstorm/debate 编排推迟到 v2。

不能只把 shared board、lane id 或可见 artifact 写入 diagnostics 就宣称完成产品语义；diagnostics 可以作为 v1 存储位置，但不能替代 orchestrator 状态机。

## 明确不做

- v1 不做一轮内复杂 DAG。
- v1 不删除现有 workflow/project/stage 模型。
- v1 不保留旧 Astra run lifecycle 字段或旧 stage-decision 调度兼容逻辑；未发布 SQLite 旧表形状直接按新 schema 修改。
- v1 不要求 workflow thread 必须使用 thread-level assistants。
- v1 不删除 workflow/project/stage 人工流程模型和人工 stage/issue API。
- 如果不实现 `brainstorm_backend` / `debate_backend` 或等价专用策略，v1 不宣称完整支持 brainstorm/debate 自动编排。
