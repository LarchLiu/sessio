# Astra Agent 解耦架构设计

## 概述

本文档描述了 Astra 编排系统从硬绑定 Astra Pi agent 到支持任意 enabled agent 的解耦重构。

## 背景

在重构前，Astra 的 planner 和 decision engine 硬编码绑定到内置的 Astra Pi agent：

* `AstraPiAcpPlanner` 和 `AstraPiAcpDecisionEngine` 通过 ACP 子进程调用 bundled Astra Pi agent

* 配置通过写入 Pi 的 `settings.json` / `models.json` 文件传递

- 无法选择其他 agent（如 Codex, Claude, Gemini）用于 planning 或 decision

这限制了灵活性，用户无法：

* 使用不同的 agent 进行 planning 和 decision

* 将 Astra 与其他 LLM providers 集成

* 在不同场景下选择最适合的 agent

## 设计目标

1. **解耦 Astra 与 Astra Pi agent**：Astra Pi agent 应该是一个可选的 backend，而不是唯一选择

2. **支持任意 enabled agent**：用户可以选择 Codex、Claude、Gemini 或任何其他支持的 agent

3. **统一 backend 接口**：所有 planner 和 decision backends 实现相同的 trait

4. **保留现有功能**：deterministic planner 作为 fallback，Astra Pi ACP 作为可选 backend

5. **向后兼容**：现有配置继续工作，默认行为不变

## 架构设计

### Backend Trait 系统

定义了两个核心 trait：

```rust
pub trait PlannerBackend: Send + Sync {
    fn plan(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        config: &Value,
    ) -> Result<BackendResponse<AstraPlan>, BackendFailure>;

    fn backend_type(&self) -> &'static str;
    fn supports_fallback(&self) -> bool;
}

pub trait DecisionBackend: Send + Sync {
    fn decide(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        result: &AstraTaskResult,
        task: &AstraTaskProposal,
        config: &Value,
    ) -> Result<BackendResponse<AstraDecision>, BackendFailure>;

    fn backend_type(&self) -> &'static str;
    fn supports_fallback(&self) -> bool;
}
```

### Backend 实现

#### 1. RuntimeAgentBackend

通过 `RuntimeManager` 调用任意 enabled agent：

* **RuntimeAgentPlanner**：使用指定的 agent 进行 planning

* **RuntimeAgentDecisionEngine**：使用指定的 agent 进行 decision making

优势：

* 可以使用任何已配置的 agent（Codex、Claude、Gemini 等）

* 复用现有的 RuntimeManager 基础设施

* 支持 agent 的所有配置选项（model、effort、permissions 等）

```rust
pub struct RuntimeAgentBackendConfig {
    pub agent: Agent,
    pub timeout_ms: u64,
    pub model: Option<String>,
    pub effort: Option<String>,
}
```

#### 2. Astra Pi ACP Backend

保留原有的 Astra Pi ACP 实现，作为一个 backend 选项：

* **AstraPiAcpPlanner**：通过 ACP 子进程调用 bundled Astra Pi agent

* **AstraPiAcpDecisionEngine**：同上

现在实现了 `PlannerBackend` 和 `DecisionBackend` trait。

#### 3. Deterministic Backend

规则驱动的确定性 backend，不依赖外部 agent：

* **DeterministicPlannerBackend**：基于规则生成任务

* **DeterministicDecisionBackend**：基于规则做决策

作为最终 fallback，当其他 backend 失败时使用。

### 配置结构

```rust
struct AstraBackendConfig {
    pub agent: Option<Agent>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    pub provider_config: AstraPiAcpProviderConfig,
}
```

### Backend 选择逻辑

在 orchestrator 中，backend 按以下优先级选择：

#### Planner Backend

1. 如果配置了 `agent`，使用 `RuntimeAgentPlanner`

2. 如果有 `astra_pi_acp_config`，使用 `AstraPiAcpPlanner`

3. 否则使用 `DeterministicPlannerBackend`

#### Decision Backend

1. 如果配置了 `agent`，使用 `RuntimeAgentDecisionEngine`

2. 如果有 `astra_pi_acp_config`，使用 `AstraPiAcpDecisionEngine`

3. 否则使用 `DeterministicDecisionBackend`

### Fallback 机制

当 backend 失败时：

* **policy_denied**：不 fallback，直接报错（用户明确拒绝）

* **timeout/transport_failure/invalid_json**：fallback 到 deterministic backend

* **deterministic backend**：永不失败（无 fallback）

## 数据流

```text
用户创建 Astra Run
    ↓
AstraService 加载 backend 配置
    ↓
Orchestrator 选择 planner backend
    ↓
┌─────────────────────────────────────┐
│ RuntimeAgentPlanner                 │ ← 如果配置了 agent
│   → RuntimeManager                  │
│   → 调用指定的 agent                │
└─────────────────────────────────────┘
         或
┌─────────────────────────────────────┐
│ AstraPiAcpPlanner                        │ ← 如果有 bundled Astra Pi
│   → ACP 子进程                      │
│   → 调用 Astra Pi agent                   │
└─────────────────────────────────────┘
         或
┌─────────────────────────────────────┐
│ DeterministicPlannerBackend         │ ← Fallback
│   → 基于规则生成计划                │
└─────────────────────────────────────┘
    ↓
生成 AstraPlan
    ↓
Orchestrator 分发任务并等待结果
    ↓
Orchestrator 选择 decision backend
    ↓
[类似的 backend 选择流程]
    ↓
应用 decision 并继续编排
```

## 实现细节

### 模块结构

```text
src-tauri/src/astra/
├── backend.rs                    # Backend trait 定义
├── runtime_agent_backend.rs      # RuntimeAgent 实现
├── astra_pi_acp_adapter.rs            # Astra Pi ACP 实现
├── deterministic_backend.rs     # Deterministic 实现
├── orchestrator.rs              # 编排器（选择 backend）
├── mod.rs                       # AstraService（配置管理）
└── ...
```

### 关键改动

1. **mod.rs**

   * 将 `astra_preferences: Mutex<AstraPiAcpProviderConfig>` 改为 `Mutex<AstraBackendConfig>`

   * 添加统一 `agent` 配置

   * 实现 `astra_backend_config()` 方法

2. **orchestrator.rs**

   * 重构 `plan_astra_round()` 使用 trait-based backend

   * 重构 `decide_astra_task()` 使用 trait-based backend

   * 添加 `create_planner_backend()` 和 `create_decision_backend()` 工厂方法

3. **astra_pi_acp_adapter.rs**

   * 为 `AstraPiAcpPlanner` 实现 `PlannerBackend` trait

   * 为 `AstraPiAcpDecisionEngine` 实现 `DecisionBackend` trait

## 使用场景

### 场景 1：使用 Claude 作为 Astra agent

```json
{
  "astra": {
    "agent": "claude",
    "provider": {
      "model": "claude-opus-4",
      "effort": "high"
    }
  }
}
```

### 场景 2：继续使用 Astra Pi agent（向后兼容）

如果不配置 `agent`，行为与之前相同：

* 如果有 bundled Astra Pi，使用 Astra Pi ACP backend

* 否则使用 deterministic backend

### 场景 3：纯 deterministic 模式

移除 bundled Astra Pi binary，不配置任何 agent，完全使用规则驱动的编排。

## 未来扩展

### 1. 前端 UI 集成

* Agent 下拉选择框（列出所有 enabled agents）

* 每个 agent 的专属配置（model, effort 等）

### 2. 混合策略

支持更复杂的 backend 选择策略：

* 根据任务类型选择不同的 agent

* Round-robin 或负载均衡

* 基于成本/延迟的动态选择

### 3. Agent-specific 优化

针对不同 agent 优化 prompt 格式：

* Claude：利用其 reasoning 能力

* Codex：优化代码相关任务

* Gemini：利用其多模态能力

### 4. Observability

增强诊断和监控：

* 记录每个 backend 的性能指标

* 比较不同 agent 的效果

* A/B testing 支持

## 兼容性

### 向后兼容性

✅ **完全向后兼容**

* 现有配置无需修改

* 默认行为不变（Astra Pi ACP → Deterministic fallback）

* 所有现有 API 保持不变

### 迁移路径

对于想要使用新特性的用户：

1. **第一步**：确保目标 agent 已启用并配置

2. **第二步**：在 Astra 配置中指定 `agent`

3. **第三步**：测试并调整配置

## 测试策略

### 单元测试

* Backend trait 实现的正确性

* Backend 选择逻辑

* Fallback 机制

### 集成测试

* RuntimeAgentBackend 与 RuntimeManager 的集成

* Astra Pi ACP backend 的兼容性

* End-to-end 编排流程

### 手动测试

* 不同 agent 组合的实际效果

* 性能和延迟对比

* UI 交互流程

## 限制和注意事项

1. **Agent 可用性**：选择的 agent 必须已启用且配置正确

2. **Prompt 兼容性**：不同 agent 对 prompt 格式的理解可能不同

3. **性能差异**：不同 agent 的响应时间可能差异较大

4. **成本考虑**：某些 agent 可能有 API 调用成本

## 总结

这次重构实现了 Astra 与 Astra Pi agent 的解耦，提供了灵活的 backend 系统：

* ✅ 支持任意 enabled agent

* ✅ 统一的 backend 接口

* ✅ 完全向后兼容

* ✅ 清晰的 fallback 机制

* ✅ 为未来扩展奠定基础

用户现在可以根据具体需求选择最合适的 agent 组合，不再受限于单一的 Astra Pi agent 实现。
