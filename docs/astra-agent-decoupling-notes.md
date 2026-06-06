# Astra Agent 解耦重构 - Release Notes

## 🎯 核心改进

### Astra 不再硬绑定 Astra Pi Agent

Astra 编排系统已重构，从硬绑定内置 Astra Pi agent 升级为支持任意 enabled agent 的灵活架构。

## ✨ 新特性

### 1. 可配置的 Astra Agent

现在可以为 Astra 的 planning 和 decision making 选择统一的 agent：

- **Astra Agent**：负责生成执行计划、评估任务结果并做出决策

支持的 agent 包括：
- Codex
- Claude
- Gemini  
- 以及任何其他 enabled 的 agent

### 2. 统一的 Backend 系统

引入了统一的 backend trait 系统：

- **RuntimeAgentBackend**：通过 RuntimeManager 调用任意 enabled agent
- **Astra Pi ACP Backend**：保留原有的 Astra Pi ACP 子进程方式（向后兼容）
- **Deterministic Backend**：规则驱动的确定性 backend（作为 fallback）

### 3. 智能 Fallback 机制

- `policy_denied` 错误不会 fallback（尊重用户决策）
- `timeout`、`transport_failure`、`invalid_json` 等错误会自动 fallback 到 deterministic backend
- 确保 Astra 编排的稳定性和可靠性

## 🏗️ 架构改进

### Backend Trait 定义

```rust
pub trait PlannerBackend: Send + Sync {
    fn plan(...) -> Result<BackendResponse<AstraPlan>, BackendFailure>;
    fn backend_type(&self) -> &'static str;
    fn supports_fallback(&self) -> bool;
}

pub trait DecisionBackend: Send + Sync {
    fn decide(...) -> Result<BackendResponse<AstraDecision>, BackendFailure>;
    fn backend_type(&self) -> &'static str;
    fn supports_fallback(&self) -> bool;
}
```

### Backend 选择优先级

#### Planner
1. 配置的 `agent` → RuntimeAgentPlanner
2. Bundled Astra Pi ACP config → AstraPiAcpPlanner
3. Fallback → DeterministicPlannerBackend

#### Decision Engine
1. 配置的 `agent` → RuntimeAgentDecisionEngine
2. Bundled Astra Pi ACP config → AstraPiAcpDecisionEngine
3. Fallback → DeterministicDecisionBackend

## 🔄 向后兼容性

✅ **完全向后兼容**

- 现有配置无需修改
- 默认行为保持不变
- 所有现有 API 继续工作
- 如果不配置新的 agent 选项，行为与之前完全一致

## 📝 使用示例

### 配置示例（未来支持）

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

### 典型场景

1. **使用 Claude 作为 Astra agent**：利用其强大的推理能力分解复杂任务并评估结果
2. **使用 Codex 作为 Astra agent**：利用其代码理解能力评估实现质量
3. **灵活切换**：根据项目特点选择最合适的 agent

## 🔧 技术细节

### 新增模块

- `backend.rs` - Backend trait 定义
- `runtime_agent_backend.rs` - RuntimeAgent 实现
- `deterministic_backend.rs` - Deterministic backend 包装
- `astra-agent-decoupling.md` - 详细架构设计文档

### 重构模块

- `orchestrator.rs` - 使用 trait-based backend 选择
- `astra_pi_acp_adapter.rs` - 实现 PlannerBackend 和 DecisionBackend trait
- `mod.rs` - 配置结构改为 AstraBackendConfig

## 🚀 未来规划

### 短期（即将支持）

- [ ] 前端 UI：在 Astra 设置页面添加 agent 选择器
- [ ] 配置持久化：保存用户的 agent 选择到数据库
- [ ] 诊断增强：记录不同 backend 的性能指标

### 中期

- [ ] Agent-specific prompt 优化
- [ ] 混合策略支持（根据任务类型动态选择）
- [ ] 成本和性能对比分析

### 长期

- [ ] A/B testing 框架
- [ ] 自动 agent 选择推荐
- [ ] Multi-agent 协作模式

## 📚 相关文档

- [架构设计文档](./docs/astra-agent-decoupling.md)
- [Pi Rust SDK 迁移计划](./docs/astra-pi-rust-sdk-migration-plan.md)

## 🙏 致谢

这次重构为 Astra 的灵活性和可扩展性奠定了坚实基础，感谢所有参与讨论和测试的用户！

---

**注意**：此版本专注于后端架构重构。前端 UI 集成和用户可见的配置界面将在后续版本中推出。
