# Sessio 架构重构总方案

**最后更新**: 2026-07-03  
**当前状态**: 阶段 1 首批薄命令模块、初始 `state/` 与初始 `window/` 拆分已落地
**本文档角色**: 唯一的重构主文档，统一记录现状、方案、实施计划和进度

---

## 1. 目标

这轮重构的目标不是统一格式或做一次“大扫除”，而是分阶段解决几个明确的工程问题：

1. 把 `src-tauri/src/lib.rs` 从巨型入口拆成可维护模块。
2. 逐步拆解 `store/sqlite.rs` 和 `SessionStore` 的职责混杂问题。
3. 在不引入行为回归的前提下，收敛配置解析和前后端类型同步。
4. 保持应用在每个阶段都能编译、运行、回退。

---

## 2. 当前代码状态

### 2.1 关键文件规模

| 文件 | 当前行数 | 说明 |
|------|----------|------|
| `src-tauri/src/lib.rs` | 9,076 | 仍然是主重构对象，首批薄命令、初始状态类型和初始窗口 helper 已迁出 |
| `src-tauri/src/store/sqlite.rs` | 15,292 | 最大的单文件风险点 |
| `src-tauri/src/store/mod.rs` | 1,104 | `SessionStore` 过大且混入默认业务逻辑 |
| `src-tauri/src/config.rs` | 1,839 | 含配置恢复与兼容解析逻辑 |
| `src/api.ts` | 2,970 | 前端手写类型很多 |
| `src-tauri/src/commands/` | 7 个模块 | 已包含首批薄命令模块与模块入口 |

### 2.2 当前重构落地情况

- `commands/` 目录已经创建，并重新落地首批薄命令模块。
- 之前对 `lib.rs` 的命令迁移曾整体回退；当前已在新基线上重新启动迁移。
- `lib.rs` 当前通过 `generate_handler![]` 注册 169 个唯一 Tauri command 名称。
- `#[tauri::command]` 在 `lib.rs` 中共出现 186 次，其中包含平台 `#[cfg]` 下的同名实现，因此不能把“186 vs 169”的差值直接理解为未注册死代码。
- `lib.rs` 中仍保留多数命令实现与大量入口层辅助逻辑，但首批薄命令实现已迁出。
- `state/` 已初步拆出 appshot shortcut 与 screenshot overlay 状态类型。
- `window/` 已初步拆出 appearance 命令、系统主题 observer、主窗口 show/hide helper。
- 已新增 `scripts/check-tauri-commands.mjs`，用于校验当前目标平台有效 `#[tauri::command]`、`generate_handler![]` 和前端 literal `invoke()` 名称集合。

### 2.3 当前目录形态

```text
src-tauri/src/
├── lib.rs
├── commands/              # 首批薄命令模块已落地
├── state/                 # appshot / screenshot overlay 状态类型已初步拆出
├── window/                # appearance 与主窗口 show/hide helper 已初步拆出
├── agents/
├── astra/
├── store/
├── memory/
└── ...
```

---

## 3. 已知约束

### 3.1 `lib.rs` 拆分约束

- Tauri 命令迁移必须是“从 `lib.rs` 删除，再在新模块中注册”，不能双写。
- 一部分 DTO 仍定义在 `lib.rs`，命令迁移时要顺手判断 DTO 是否也该一起挪走。
- 全局状态、窗口逻辑、截图逻辑仍深度耦合在入口层，后续拆分时要避免事件链断裂。
- 前端在 `src/api.ts`、`src/theme.ts`、`src/App.tsx` 等位置大量使用 `invoke("command_name")` 字符串调用，命令名或参数形态漂移不能只靠 Rust 编译发现。
- 首批迁移应优先选择“薄命令层”，也就是主要做参数转换与 store/service 调用的命令。
- `computer_use`、`appshot`、screenshot overlay、tray、global shortcut 不适合作为第一批迁移样板，它们应后置处理。

### 3.2 `store/sqlite.rs` 拆分约束

- 当前 `SessionStore` 不只覆盖 `session / project / thread / astra / canvas / memory`。
- 它还包含 `channel sessions`、`scheduled tasks`、`process templates`、`assistants`、`runtime preferences`、`project stages / kanban` 等职责。
- trait 内已经混入默认业务逻辑，例如 `get_thread_replay()`。
- 所以不能简单假设“拆成 6 个 trait 就结束”。

### 3.3 `config.rs` 收敛约束

- 当前配置层不只是“手写 TOML 解析器”。
- 它还承担：
  - 损坏配置恢复与备份通知
  - 宽松布尔和 `null` 语义
  - 忽略未知 section
  - structured MCP sections 兼容
- 因此不能直接用 `toml::from_str()` 一步替换全部逻辑。

### 3.4 类型同步约束

- DTO 不只在 `models.rs`。
- 还散落在 `lib.rs` 和 `commands/*`。
- `api.ts` 的手写类型较多，`ts-rs` 只能分批引入，不能一次性替换完。

---

## 4. 目标结构

目标不是一次性达到理想架构，而是按可验证的小步演进到下面的结构：

```text
src-tauri/src/
├── lib.rs                    # 入口、模块声明、应用初始化
├── commands/                 # 所有 Tauri commands
├── state/                    # 应用级状态
├── window/                   # 窗口创建与事件处理
├── agents/
├── astra/
├── store/
│   ├── mod.rs
│   ├── cached.rs
│   └── sqlite/
│       ├── mod.rs
│       ├── sessions.rs
│       ├── projects.rs
│       ├── threads.rs
│       ├── scheduled_tasks.rs
│       ├── assistants.rs
│       ├── stages.rs
│       └── ...
└── ...
```

---

## 5. 分阶段实施方案

### 阶段 1: 拆分 `lib.rs`

**目标**: 在命令迁移已回退的基线上，从零重新启动 `lib.rs` 拆分，并优先建立稳定的命令模块边界。

#### 5.1 当前基线判断

- [x] 创建 `commands/` 模块结构
- [x] 首批薄命令迁移已重新开始
- [x] 首批迁移命令已通过模块路径注册到 `generate_handler![]`
- [x] `state/` 初始拆分已落地
- [x] `window/` 初始拆分已落地

当前这一阶段不应该被理解为“继续完成上次已开始的命令迁移”，而应该被理解为“在新的基线下重新定义拆分顺序并重新启动”。

#### 5.2 上次回退后已确认的教训

当前还不能精确还原上次每一个回退决策的即时背景，但从现有代码状态和回退结果，已经可以确认以下几条教训应写入本轮计划：

1. 上次推进前没有先定义“首批迁移完成”的退出条件，导致何时继续拆 `state/`、`window/`，何时先回稳，缺少共同判断标准。
2. 命令迁移主要依赖人工核对，缺少“命令定义集合 vs handler 注册集合”的机械校验，导致一旦出现遗漏或双写，排查和回退成本都偏高。
3. 前端 `invoke()` 字符串调用没有被纳入显式验证闭环，Rust 层通过并不等于前端调用点安全。
4. 当 `lib.rs` 命令迁移、入口依赖整理和后续结构重排同时发生时，变形面过大，不利于快速定位回归来源。

因此，这一轮阶段 1 的重点不是“尽快搬更多文件”，而是先建立一套可验证、可暂停、可回退的迁移节奏。

#### 5.3 第一批推荐迁移对象

第一批只迁移“薄命令层”，即主要承担参数整理、错误映射和 store/service 调用的命令，不优先碰截图、窗口、权限、shortcut、runtime session 生命周期、外部进程调用或文件系统副作用较重的命令。

推荐模块边界如下，但这些模块名只是目标归宿，不代表一次搬完整领域文件：

1. `commands/sessions.rs`
2. `commands/projects.rs`
3. `commands/process_templates.rs`
4. `commands/assistants.rs`
5. `commands/astra.rs`
6. `commands/settings.rs`
7. `commands/kanban.rs`

首批更建议直接按具体函数选择，而不是按领域模块整体搬迁。优先候选：

- `sessions`: `list_sessions`、`list_channel_sessions`、`update_session_rename_title`
- `projects`: `list_projects`、`add_existing_project`、`update_project`、`archive_project`
- `process_templates`: `list_process_templates`、`create_process_template`、`update_process_template`、`delete_process_template`
- `assistants`: `list_assistants`、`create_assistant`、`update_assistant`、`delete_assistant`
- `astra`: `get_astra_config`、`update_astra_config`、`create_astra_run`、`cancel_astra_run`、`list_astra_runs`、`get_astra_run`
- `settings`: `get_debug_config`、`get_network_config`、`update_network_config`、`get_mcp_settings`、`get_appshot_config`、`take_config_recovery_notice`
- `kanban`: `list_kanban_items`、`create_kanban_item`、`update_kanban_item`、`update_kanban_item_status`、`delete_kanban_item`

第一批明确后置的同领域命令：

- `agents` 中的 runtime session 生命周期命令：`start_agent_session`、`fork_agent_session`、`ensure_agent_runtime_session`、`load_agent_session`、`send_agent_input`、`cancel_agent_turn`、`set_agent_session_config_option`、`respond_agent_permission`
- `canvas` 中带文件写入、revision pruning 或上下文文件创建的命令：`save_canvas_draft`、`save_canvas_revision`、`create_canvas_context_file`
- `memory` 中会构造 memory service 或调用 qmd backend 的命令：`get_memory_backend_status`、`search_project_memory`
- `create_default_project` 这类带默认目录创建等文件系统副作用的项目命令

每迁一组，都同步清理 `generate_handler![]` 和 `use` 依赖，避免只搬函数不减入口复杂度。

#### 5.4 首批完成的退出条件

只有在以下条件同时满足后，才把“阶段 1 首批迁移”视为稳定，可继续推进 `state/`、`window/` 或并行准备 sqlite 重构：

1. `5.3` 的具体函数候选集合中至少 3 个薄命令模块已经落地，且对应命令实现与注册已从 `lib.rs` 删除，不保留同名双写。
2. Rust 命令注册一致性校验通过：`generate_handler![]` 中的唯一命令名称集合与当前目标平台实际启用的 `#[tauri::command]` 定义集合一致，确认既无漏注册也无重复注册。
3. 该校验必须是 cfg-aware：不能用 raw `#[tauri::command]` 出现次数直接和 handler 数量比较；同名平台分支实现应按目标平台展开后的有效定义计算。
4. 前端命令调用静态 diff 校验通过：全 `src/**/*.ts(x)` 中 literal `invoke("command_name")` 的唯一名称集合必须全部存在于 `generate_handler![]`，handler-only 名称必须显式记录为允许的非 `api.ts` 调用或暂未暴露命令。
5. 对涉及命令迁移的前端调用执行 `pnpm run build` 或 `pnpm run check` 通过；这只能作为类型和打包验证，不能替代第 4 条的字符串命令名 diff。
6. 已迁移命令组完成最小手动回归，至少覆盖对应的主流程调用路径与错误提示路径。

辅助量化指标可以用于观察阶段 1 是否真正开始收敛，但不单独构成通过条件：

- `commands/` 中已经出现首批稳定模块，而不是只有空目录。
- `lib.rs` 相对当前 9,720 行基线开始持续下降；建议先把 `< 9,000` 作为第一阶段是否见效的观察点，而不是硬门槛。

#### 5.5 后置的高耦合区域

以下内容不建议作为阶段 1 开局样板，应后置到命令模块初步稳定之后：

1. `computer_use`
2. `appshot`
3. screenshot overlay
4. tray / 主窗口事件
5. global shortcut / permission 面板

这些逻辑和 `AppHandle`、窗口生命周期、平台权限、异步事件流强耦合，拆分时更适合连同 `state/`、`window/` 一起处理。

#### 5.6 随后拆出的非命令逻辑

1. `state/`
   - `AppshotShortcutState`
   - `ScreenshotOverlayState`
   - 其他全局状态
2. `window/`
   - 主窗口管理
   - 窗口事件处理
   - 截图叠加层窗口逻辑

### 阶段 2: 重构 `store/sqlite.rs`

**目标**: 先重画职责边界，再拆 trait 和实现文件。

#### 5.7 当前判断

当前 `store` 层的问题不只是 `sqlite.rs` 过大，而是三类职责被挤在了同一个文件与同一个 trait 里：

1. schema / migration / seed 初始化
2. session identity merge 与 sidebar 可见性规则
3. thread / stage / plan / astra / canvas 等业务域的持久化与聚合查询

因此这一阶段的首要目标不是“把一个大文件切成很多小文件”，而是先把规则边界画清楚，避免拆完以后仍然到处回穿。

#### 5.8 SQLite 子文档: 详细重构实施方案

##### 5.8.1 当前代码事实

- `src-tauri/src/store/sqlite.rs` 约 15k 行，是当前最大的单文件风险点。
- `src-tauri/src/store/mod.rs` 中的 `SessionStore` 约 120+ 个方法，范围已经覆盖：
  - sessions / indexed sessions / subagents
  - channel sessions
  - scheduled tasks
  - process templates / projects / assistants
  - threads / stages / kanban
  - plan rounds / tasks / task sessions
  - astra runs
  - runtime agent capability / config
  - session history snapshots / thread work snapshots
  - canvas
- `SessionStore` 里还带有默认业务逻辑：
  - `get_thread_replay()`
  - `create_thread_with_origin()`
  - `update_thread_with_options()`
- `CachedStore` 不只是转发层，而是维护了 indexed-session 的内存快照，并对以下行为做增量修补：
  - `upsert_session`
  - `replace_by_scope`
  - `upsert_subagent`
  - `mark_missing_scopes_unavailable`
  - `cleanup_partial_astra_sessions`

##### 5.8.2 当前最强耦合点

1. `session identity merge` 已经形成独立规则系统
   - 典型逻辑包括 placeholder 与 real row 合并、`origin` sticky 升级、`scheduled_task_id` sticky 保留、`is_auxiliary` sticky-OR。
   - 这些规则集中在 `insert_session()` 及其辅助函数附近，不适合散落到多个 repository 中各自维护。
2. workflow 区域存在跨域聚合
   - `load_thread_by_id()` 会继续 hydrate assistants / agents / stages / sessions。
   - `load_thread_stages()` 会继续加载 stage assistants / sessions / issues，并补 thread stage 默认状态。
   - `list_thread_index()` 会横跨 thread / stage / plan / astra / session 聚合活动时间与 session key。
   - `get_thread_replay()` 也是跨 thread / plan / astra / session 的聚合视图。
3. session 可见性与 workflow 绑定在一起
   - `link_thread_session()`、`link_stage_session()`、`link_plan_task_session()`、`interrupt_active_astra_runs()` 都会影响 session 的 sidebar 可见性或 placeholder 生命周期。
4. cache 与 store 的边界尚未显式建模
   - 目前 cache 只缓存 indexed-session 视图，但它已经内置了 virtual session、placeholder、subagent 等特殊语义。

##### 5.8.3 明确不建议的做法

- 不建议第一步就拆 `SessionStore` 为很多小 trait。
- 不建议按“每张表一个文件”机械拆分。
- 不建议先移动实现、后统一 session 规则。
- 不建议忽略 `CachedStore`，假设它会自然适配后续重构。

##### 5.8.4 推荐的目标边界

第一层不追求最终理想架构，而是先建立稳定的中间层边界：

1. 公共规则层
   - `session_rules.rs`
   - 放置 `is_real_session_file_path`、best-session 选择规则、virtual session 判断、时间与文件元数据工具函数
2. session identity 层
   - `session_identity.rs`
   - 放置 `insert_session()` 及 placeholder / lineage / provenance merge 规则
3. schema/bootstrap 层
   - `schema.rs`
   - `migrations.rs`
   - `seed.rs`
4. workflow query/service 层
   - `thread_replay.rs`
   - `thread_index.rs`
   - 这里处理跨 thread / stage / plan / astra 的聚合查询
5. domain persistence 层
   - `sessions.rs`
   - `projects.rs`
   - `assistants.rs`
   - `threads.rs`
   - `stages.rs`
   - `kanban.rs`
   - `plans.rs`
   - `astra.rs`
   - `scheduled_tasks.rs`
   - `channel_sessions.rs`
   - `runtime_agents.rs`
   - `canvas.rs`
   - `snapshots.rs`

##### 5.8.5 中间态 trait 设计

不建议一上来就把 public API 细分到十几个 trait。更稳妥的中间态是先按能力分组，例如：

- `SessionCatalogStore`
- `WorkflowStore`
- `ProjectConfigStore`
- `ScheduledTaskStore`
- `ChannelSessionStore`
- `RuntimeAgentStore`
- `CanvasStore`
- `SnapshotStore`

`MemoryStore` 继续独立，不与这轮 sqlite 重构混在一起。

在中间态里可以保留一个兼容性的聚合 trait 或 type alias，让 `lib.rs` 与未来的 `commands/*` 暂时仍能用统一入口，等命令层逐步收窄依赖以后，再决定是否完全取消大 trait。

##### 5.8.6 推荐落地顺序

建议在阶段 1 满足“首批完成的退出条件”后，再并行推进这一阶段。这样做的原因不是 sqlite 重构依赖命令模块本身，而是要先把 `lib.rs` 迁移节奏稳定下来，避免入口层与 store 层同时大范围变形，导致回归面失控。

**第 1 步: 先补 cache 回归测试**

- 为 `CachedStore` 增补以下场景测试：
  - [x] placeholder 被 real row 替换
  - [x] `replace_by_scope` 后 subagent 保留
  - [x] `mark_missing_scopes_unavailable` 不误伤 virtual session
  - [x] astra placeholder cleanup 后 snapshot 刷新

**第 2 步: 抽公共规则，不改行为**

- 从 `store/mod.rs`、`sqlite.rs`、`cached.rs` 抽出重复规则：
  - [x] `is_real_session_file_path`
  - [x] best-session 选择规则
  - [x] virtual session 判断
  - [x] placeholder indexed-session 判断
  - [x] `now_ms`
  - [x] `file_mtime_for`
- 这一步的目标是消除重复实现，先统一行为，再拆模块。

**第 3 步: 拆 schema / migration / seed**

- 把以下内容从 `sqlite.rs` 中移出：
  - [ ] `SCHEMA_SESSIONS`
  - [ ] `SCHEMA_MEMORY`
  - [ ] `SCHEMA_APP`
  - [ ] `initialize_schema()`
  - [x] `ensure_column()`
  - builtin seeds 相关逻辑
- 这是最适合先落刀的一层，因为对上层业务行为影响最小。

**第 4 步: 拆 session identity 子系统**

- 重点迁移：
  - `load_identity_session_rows()`
  - provenance merge helpers
  - `insert_session()`
  - `mark_session_scheduled_task()`
  - `mark_session_origin()`
  - `replace_by_scope()`
- 这一层要被视为“规则子系统”，而不是普通 CRUD。

**第 5 步: 抽 workflow 聚合查询**

- 优先抽出：
  - `thread_replay`
  - `thread_index`
  - `load_thread_by_id()` 相关 hydrate helpers
  - `load_plan_round_by_id()` / `load_plan_tasks()` 等组合查询
- 目标是先把“聚合视图”从“基础持久化”里分开。

**第 6 步: 再按领域搬 persistence 实现**

- 在规则层与聚合层稳定后，再拆 domain modules：
  - `scheduled_tasks`
  - `channel_sessions`
  - `projects`
  - `assistants`
  - `threads`
  - `stages`
  - `plans`
  - `astra`
  - `runtime_agents`
  - `canvas`
  - `snapshots`

**第 7 步: 最后再收窄 public trait**

- 等 sqlite 内部边界稳定后，再让 `lib.rs` 和 `commands/*` 逐步从 `Arc<dyn SessionStore>` 收敛到更小的能力接口。
- 这一步不应该早于内部模块稳定。

##### 5.8.7 `CachedStore` 兼容策略

`CachedStore` 需要被当成这一阶段的一级对象处理，而不是最后顺手修。

建议策略：

1. 把它明确定位为 `indexed session view cache`。
2. 让它只依赖最小必需能力：
   - `list_indexed_sessions`
   - `upsert_session`
   - `replace_by_scope`
   - `upsert_subagent`
   - 文件可用性更新
   - astra placeholder cleanup
3. 把 cache 中的特殊语义与 sqlite 公共规则复用同一份 helper，避免再次复制 placeholder / virtual-session 判定逻辑。

##### 5.8.8 风险点与停止条件

高风险点：

- session visibility 回归，导致 sidebar 显示异常
- placeholder / real row 合并规则漂移
- thread replay / thread index 聚合结果变化
- `CachedStore` snapshot 与 sqlite 真值源不一致
- trait 过早拆分，导致 `lib.rs` 和未来的 `commands/*` 改动面失控

每一小步都应满足以下停止条件后再继续下一步：

- Rust 编译通过
- `store/sqlite.rs` 相关测试通过
- 新增的 cache 测试通过
- 文档同步更新

##### 5.8.9 阶段 2 的完成标准

阶段 2 不以“`sqlite.rs` 已经很小”作为唯一完成标准，而是同时满足：

1. schema / session rules / workflow aggregation / domain persistence 已分层
2. `SessionStore` 不再承载明显的默认业务编排逻辑
3. `CachedStore` 的职责边界清晰且测试充分
4. 命令层开始使用更小的能力接口，而不是继续无限依赖单个巨型 trait

辅助量化指标可作为阶段 2 是否真正收敛的观察值：

- `src-tauri/src/store/sqlite.rs` 不再继续维持当前约 15k 行的单文件形态；建议先把“主文件降到 `< 8,000` 且拆出至少 4 个稳定子模块”作为第一观察点。
- schema / seed / migration、session identity、workflow aggregation 三类内容至少已分别落到独立文件，而不是仍混在同一实现单元。
- `SessionStore` 或其中间态聚合接口的方法数量相对当前基线出现下降趋势；建议先把“核心聚合接口 `< 90` 个方法”作为观察值，而不是硬门槛。

### 阶段 3: 收敛配置管理

**目标**: 保留兼容行为的前提下，把解析逻辑迁到更标准的实现上。

#### 5.9 Config 子文档: 详细重构实施方案

##### 5.9.1 当前代码事实

- `src-tauri/src/config.rs` 约 1.8k 行，但它不是普通的“配置 struct + serde 读写”文件，而是一个完整的配置子系统。
- 当前至少混合了以下 5 层职责：
  1. 加载入口与双模式读取
  2. 自定义 parser
  3. `RawConfig -> AppConfig` 的 resolve 逻辑
  4. 默认值补全与写回
  5. canonical 序列化输出
- 当前存在两条读取路径：
  - `load_config()`：宽松模式，配置无效时保留原文件、发 recovery notice、本次运行退回默认值
  - `load_config_strict()`：严格模式，直接报错，供 watcher 等场景使用
- 当前 parser 实际上定义了一套“类 TOML”语义，而不是真正完全交给 `toml::from_str()`：
  - 宽松布尔值
  - `null -> None`
  - inline comment stripping
  - 字符串数组解析
  - unknown section 忽略
- `computer_use` 的最终配置不是单一 section 解析结果，而是 `[computer_use]` 与 `[mcp_servers.computer_use]` 的合成结果。
- 当前外部调用面较大，`network`、`mcp`、`computer_use`、`cli`、`config_watch`、`lib.rs` 都直接依赖 `load_config()` / `save_config()` / `take_config_recovery_notice()`，所以这轮优先做内部结构重构，不优先改外部 API。

##### 5.9.2 为什么不能直接改成 `serde + toml`

当前实现里有几类行为不是直接替换成 `toml::from_str()` 就能无损保留的：

1. 宽松布尔语义
   - 当前接受 `true/false` 之外的 `1/0/yes/no/on/off`。
2. `null` 语义
   - 当前允许显式写 `null` 来表示 `None`。
3. 兼容与容错语义
   - 当前会忽略 unknown sections。
   - 当前会继续接受并忽略 legacy `astra` section。
4. 跨 section 联合解析
   - `computer_use` 同时依赖自身 section 和 builtin MCP server section。
5. 恢复语义
   - 配置损坏时不会覆盖原文件，而是发 recovery notice 并在本次运行退回默认值。
6. 双读取模式
   - `load_config()` 与 `load_config_strict()` 的语义不同，不能简单合并。

因此这一阶段的目标不是“删除手写 parser”，而是“先分层，再逐步收敛 parser 的责任范围”。

##### 5.9.3 推荐的目标边界

建议把当前 `config.rs` 先按职责拆成以下内部模块：

1. `loader.rs`
   - `load_config`
   - `load_config_strict`
   - `load_config_from_path`
   - `finalize_loaded_config`
   - `recover_invalid_config`
2. `raw.rs`
   - `RawConfig`
   - `Raw*Config`
3. `parser.rs`
   - `parse_raw_config`
   - `parse_section`
   - `parse_value`
   - `parse_bool`
   - `parse_string_array`
   - `strip_comment`
4. `resolver.rs`
   - `resolve_app_config`
   - `resolve_mcp_config`
   - `resolve_computer_use_config`
   - `resolve_memory_config_inner`
5. `defaults.rs`
   - `default_app_config`
   - `raw_config_with_defaults`
   - merge helpers
6. `serializer.rs`
   - `serialize_app_config`
   - `serialize_*`

第一阶段保持当前 public API 不变：

- `load_config()`
- `load_config_strict()`
- `save_config()`
- `save_memory_config()`
- `take_config_recovery_notice()`

##### 5.9.4 推荐实施顺序

**第 1 步: 先补 characterization tests**

- 不新增功能，先把当前关键兼容行为钉死：
  - ignores unknown sections
  - ignores legacy `astra` sections
  - invalid config recovery notice
  - `computer_use` 与 builtin MCP server 联合解析
  - default backfill 与 roundtrip 序列化

**第 2 步: 先做纯搬运式模块拆分**

- 仅把 loader / parser / resolver / defaults / serializer / raw struct 分文件。
- 目标是让 `config.rs` 先从“一个大文件”变成“清晰分层”，而不是先改协议。

**第 3 步: 重构默认值补全层**

- 当前默认值补全依赖“默认配置先序列化、再反解析”的路径。
- 这条路径虽然统一，但结构比较绕，后续应改成更直观的 typed default merge。
- 这一步仍应保持外部行为与写回结果尽量不变。

**第 4 步: 先把简单 section 收敛到 serde 驱动**

- 适合优先收敛的 section：
  - `[index]`
  - `[network.proxy]`
  - `[appshot]`
  - `[debug]`
- 这些 section 的语义简单、跨字段约束少，最适合作为 serde 化的第一步。

**第 5 步: 对复杂 section 保留 compatibility adapter**

- 暂时保留自定义适配逻辑的部分：
  - `memory`
  - `mcp_servers`
  - `computer_use`
- 先把它们包进更清晰的 resolver / compatibility 层，而不是急着完全交给通用 parser。

**第 6 步: 最后再评估是否需要彻底删除手写 parser**

- 只有当以下行为都已有稳定替代方案时，才考虑删除旧 parser：
  - 宽松布尔
  - `null`
  - unknown section ignore
  - legacy `astra` ignore
  - recovery notice 语义

##### 5.9.5 必须保留的行为

这一阶段无论内部怎么拆，以下行为都应被视为兼容契约：

1. 配置损坏时保留原文件，不自动覆盖。
2. 发出 `ConfigRecoveryNotice`，并带上尽可能准确的错误上下文。
3. `load_config()` 与 `load_config_strict()` 继续保持不同语义。
4. unknown sections 继续忽略，不因 schema 收紧而让旧配置突然失效。
5. legacy `astra` sections 继续被接受并忽略。
6. `computer_use` builtin MCP server 语义继续存在。
7. memory qmd 的环境变量覆盖行为继续保留。

##### 5.9.6 风险点与停止条件

高风险点：

- 配置写回变成 lossy rewrite，导致用户配置被悄悄改形
- `config_watch` 因 strict / tolerant 语义漂移而误报
- `computer_use` 与 MCP builtin server 行为偏移
- CLI 输出的配置文本发生不必要漂移
- 配置损坏恢复路径被破坏，导致原文件被覆盖

每一小步都应满足以下停止条件后再继续：

- Rust 编译通过
- `config.rs` 自带测试通过
- `network` / `mcp` / `computer_use` 相关最小回归通过
- 严格模式与宽松模式行为仍能清楚区分
- 文档同步更新

##### 5.9.7 阶段 3 的完成标准

阶段 3 的完成不以“已经全面换成 serde”作为唯一标准，而是同时满足：

1. parser / resolver / defaults / recovery / serializer 已分层
2. 外部配置 API 基本保持稳定
3. 关键兼容行为已有测试覆盖
4. 简单 section 已可以用更标准的解析方式维护
5. 复杂 section 已被收敛到清晰的 compatibility / resolver 边界，而不是继续散落在单文件里

辅助量化指标可作为阶段 3 是否真正改善可维护性的观察值：

- `src-tauri/src/config.rs` 不再维持当前约 1.8k 行的全集中实现；建议先把“主入口文件 `< 1,000` 行，且 loader / parser / resolver / serializer 至少已有独立文件”作为第一观察点。
- characterization tests 至少覆盖：invalid recovery、unknown section ignore、legacy `astra` ignore、`computer_use` 联合解析、roundtrip 序列化 这 5 类兼容语义。
- 第一批 serde 化的简单 section 至少包含 2 个稳定 section，再决定是否继续扩大范围。

### 阶段 4: 前后端类型同步

**目标**: 先减少最值得消除的重复类型，再评估是否全量引入 `ts-rs`。

#### 5.10 推荐顺序

1. 盘点 `models.rs`、`lib.rs`、`commands/*` 中的可导出 DTO。
2. 先为稳定共享模型接入 `ts-rs`。
3. 在 `api.ts` 中只替换已验证的重复类型。
4. 不把“删光手写类型”作为第一阶段目标。

阶段 4 的辅助量化指标：

- 优先让“高复用且已稳定”的共享 DTO 先同步，而不是追求一次性覆盖全部前后端类型；建议先完成 `5~10` 个核心 DTO 的生成与替换验证。
- `src/api.ts` 的手写类型行数应开始下降，但不把“单文件必须压到某个固定行数”作为阶段通过条件。

### 阶段 5: 后续优化

这一阶段不是当前阻塞项，优先级低于前四阶段：

- 分离 `astra/mod.rs` 的服务实现
- 提取 `claude/parser.rs` 与 `codex/parser.rs` 的共享逻辑
- 进一步演进为更清晰的分层架构

---

## 6. 当前进度

### 6.1 已落地代码

| 任务 | 状态 | 说明 |
|------|------|------|
| `commands/` 目录 | ✅ | 已创建模块入口和首批薄命令模块 |
| `lib.rs` 命令迁移 | ⏳ | 已迁出 sessions / projects / process_templates / assistants / kanban / settings 的首批薄命令 |
| `generate_handler![]` 收敛 | ⏳ | 首批迁移命令已改用 `commands::*` 模块路径注册，整体注册仍集中在 `lib.rs` |
| `state/` 拆分 | ⏳ | 已拆出 appshot shortcut 与 screenshot overlay 状态类型，后续可继续收窄状态操作方法 |
| `window/` 拆分 | ⏳ | 已拆出 appearance 命令、系统主题 observer、主窗口 show/hide helper，窗口创建与 overlay 窗口流程仍在 `lib.rs` |
| `CachedStore` 回归测试 | ✅ | 已覆盖 placeholder 替换、`replace_by_scope` subagent 保留、virtual session guard、astra cleanup snapshot refresh |
| 公共 session 规则 | ✅ | 已抽出 `session_rules.rs`，统一时间、文件 mtime、real/virtual session、placeholder indexed-session 与 best-session 选择规则 |
| SQLite schema/bootstrap | ⏳ | 已建立 `store/sqlite/schema.rs` 并迁出 `ensure_column()`；schema SQL 与 seed 逻辑仍待拆分 |

### 6.2 当前焦点

当前优先级按顺序是：

1. 进入阶段 2 第 3 步，拆分 schema / migration / seed 初始化逻辑。
2. 维持 `scripts/check-tauri-commands.mjs` 作为命令迁移的固定验证闭环。
3. 后续再评估是否继续拆 screenshot overlay 窗口流程。

---

## 7. 实施规则

### 7.1 通用原则

1. 小步快跑，每批改动都应该能独立验证。
2. 先迁移，再清理，不做跨多个主题的大杂烩提交。
3. 文档必须跟随代码状态更新，不提前宣称未来结构“已完成”。
4. 高风险模块优先做边界设计，再动手搬文件。
5. 量化指标用于辅助判断“是否在收敛”，不作为唯一完成标准；若指标好看但边界仍混乱，应以结构质量和回归风险判断为准。

### 7.2 每个任务完成后必须检查

- [ ] `cargo fmt`
- [ ] `cargo clippy`
- [ ] 相关 Rust 测试
- [ ] Rust 命令注册一致性校验通过：cfg-aware 的 `#[tauri::command]` 唯一有效名称集合与 `generate_handler![]` 唯一名称集合一致
- [ ] 前端 literal invoke 静态 diff 通过：全 `src/**/*.ts(x)` 的 `invoke("command_name")` 名称集合都在 `generate_handler![]` 中，handler-only 名称已显式说明
- [ ] 若任务涉及命令、DTO 或前端调用面，执行 `pnpm run build` 或 `pnpm run check`
- [ ] 相关手动功能回归
- [ ] 文档状态同步

### 7.3 Commit 规范

格式：

```text
<type>: <subject>

<body>
```

推荐 `type`：

- `refactor`
- `fix`
- `test`
- `docs`
- `chore`
- `deps`

---

## 8. 维护方式

- 这份文档是唯一的重构主文档。
- 后续不再拆分成独立的架构、进度、计划三份文件。
- 每完成一个阶段，只更新这份文档中的“当前代码状态”和“当前进度”两部分。
