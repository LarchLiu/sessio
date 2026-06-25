# Agent 会话索引层方案：SQLite 优先，Watcher 增量同步，预留 PostgreSQL

## Summary

当前实现里，`src-tauri/src/lib.rs` 的 `list_sessions()` 会直接调用 `readers::list_all()`，每次启动或读取时全量扫描各 agent 的会话目录并解析文件。这个优化方案建议采用：

* 保留现有各 agent reader 作为文件真相源

* 新增一个本地 SQLite 索引层，持久化 `SessionInfo` / `SubagentInfo` 级别的元数据

* 新增文件夹 watcher，监听 agent 目录变化，按文件粒度增量重解析并更新索引

* 前端列表页默认读 DB，不再每次启动全量扫盘

* 详情页消息继续按需读取原始 `json` / `jsonl` 文件，不做全文入库

* 存储抽象预留成 `Store` 接口，第二阶段可接 PostgreSQL，但第一版只实现 SQLite

这样能把风险压在最低，同时解决启动速度和运行期间实时更新体验。

## Key Changes

### 1. 后端分层

新增三层职责，避免把 DB 逻辑直接塞进现有 reader：

* `readers/*`

  * 保持现状，继续负责各 agent 文件格式解析

  * 补充按文件解析单条 session、按目录根枚举监控路径的统一接口

* `indexer`

  * 负责冷启动全量建索引、watcher 增量同步、去重、删除与归档处理

  * 把文件事件转换为“重建某个 session / 删除某个 session / 重扫某个 project 目录”

* `store`

  * 负责 SQLite 读写

  * 暴露统一接口，后续可新增 PG 实现

建议新增模块：

* `src-tauri/src/store/mod.rs`

* `src-tauri/src/store/sqlite.rs`

* `src-tauri/src/indexer/mod.rs`

* `src-tauri/src/watch/mod.rs`

### 2. 数据库存储范围

第一版只缓存列表、分组、筛选所需的索引字段，不缓存消息全文。

建议表结构：

* `sessions`

  * `agent`

  * `session_id`

  * `project_path`

  * `project_name`

  * `started_at`

  * `updated_at`

  * `message_count`

  * `first_user_message`

  * `file_path`

  * `file_size`

  * `file_mtime`

  * `file_inode_or_fingerprint`

  * `partial`

  * `available`

  * `archived`

  * `last_indexed_at`

  * 主键建议 `(agent, session_id, file_path)` 或单独 `session_key`

* `subagents`

  * `parent_agent`

  * `parent_session_id`

  * `subagent_id`

  * `agent_type`

  * `description`

  * `started_at`

  * `updated_at`

  * `message_count`

  * `first_user_message`

  * `file_path`

  * `file_size`

  * `file_mtime`

  * `partial`

* `watched_files`

  * `path`

  * `agent`

  * `kind`（`main` / `session` / `index` / `subagent` / `logs`）

  * `last_seen_mtime`

  * `last_seen_size`

  * `last_hash_optional`

  * `last_indexed_at`

* `schema_migrations`

  * 版本管理

索引建议：

* `sessions(updated_at desc)`

* `sessions(agent, updated_at desc)`

* `sessions(project_path, updated_at desc)`

* `subagents(parent_agent, parent_session_id)`

### 3. 启动与同步流程

启动流程建议定义为：

1. App 启动时先初始化 SQLite

2. 若 DB 为空或 schema 版本变化，执行一次全量建索引

3. 若 DB 已存在，先直接从 DB 返回列表给前端

4. 后台启动 watcher

5. watcher 收到变化后，做防抖和批处理，再增量更新 DB

6. DB 更新完成后向前端发送事件，前端重新拉取 `list_sessions`

冷启动全量建索引策略：

* `Codex`：扫描 `~/.codex/sessions` 与 `~/.codex/archived_sessions`

* `Claude`：扫描 `~/.claude/projects/*`

增量更新策略按 agent 区分：

* `Codex`

  * 单个 `.jsonl` 文件变化时，只重解析该文件

  * 删除时将对应 session 标记为 `available=false`，必要时 `archived=true`

* `Claude`

  * `sessions-index.json` 变化时，重扫该 project 目录

  * 某个 session `.jsonl` 变化时，只重解析该 session

  * `subagents/` 下变化时，重建对应父 session 的 subagent 列表

### 4. Watcher 设计

建议使用 Rust 文件监听库做递归监控，并加一层事件聚合：

* 监听根目录

  * `~/.codex/sessions`

  * `~/.codex/archived_sessions`

  * `~/.claude/projects`

* 事件处理策略

  * 300-800ms 防抖

  * 同一路径短时间多次变更合并

  * 对 Claude 这类“一个目录影响多条 session”的格式，按目录整体重算

* 一致性策略

  * watcher 只负责触发重建，不在事件里直接做细粒度 patch

  * 真正入库前重新读取磁盘，避免拿到半写入文件

  * 对解析失败采用“保留旧索引 + 记录错误日志 + 下次事件重试”

### 5. 前端与 Tauri 接口调整

现有前端 `src/App.tsx` 只在挂载时调用一次 `listSessions()`，需要补上事件驱动刷新。

Tauri 命令调整：

* 保留 `list_sessions`

  * 语义改为“从 DB 查询会话索引”

* 保留 `get_session_messages`

  * 继续直读原文件

* 新增 `rebuild_session_index`

  * 手动全量重建索引，作为兜底入口

* 可选新增 `get_index_status`

  * 返回 `ready` / `indexing` / `lastSync` / `error`

Tauri 事件：

* `sessions_index_updated`

  * watcher 或重建完成后发出

  * 前端收到后重新调用 `listSessions()`

* 可选 `sessions_index_status`

  * 用于展示“首次索引中 / 后台同步中”

前端改动：

* `App.tsx` 初次加载仍调用 `listSessions()`

* 增加对 `sessions_index_updated` 的监听

* 首次启动若 DB 尚未准备好，显示轻量 loading 状态

* 不改 `SessionDetail.tsx` 的消息读取路径，避免第一版范围膨胀

## Public APIs / Interfaces / Types

建议新增内部 Rust 抽象：

```rust
trait SessionStore {
    fn init(&self) -> Result<()>;
    fn list_sessions(&self) -> Result<Vec<SessionInfo>>;
    fn upsert_sessions(&self, sessions: &[SessionInfo]) -> Result<()>;
    fn replace_subagents(
        &self,
        agent: Agent,
        session_id: &str,
        items: &[SubagentInfo],
    ) -> Result<()>;
    fn delete_by_file_path(&self, path: &Path) -> Result<()>;
    fn mark_unavailable_by_file_path(&self, path: &Path) -> Result<()>;
}
```

建议新增内部索引任务接口：

```rust
enum IndexTask {
    FullRebuild,
    ReindexPath(PathBuf),
    ReindexClaudeProject(PathBuf),
    RefreshProjectMappings,
}
```

前端 TypeScript 可选新增：

* `IndexStatus`

  * `state: "ready" | "indexing" | "error"`

  * `lastSyncAt: number | null`

  * `message: string | null`

现有 `SessionInfo` / `SubagentInfo` 字段可保持不变，避免 UI 大面积改动。

## Test Plan

### Rust 单元测试

* SQLite schema 初始化与 migration

* `upsert_sessions` / `replace_subagents` / 删除与 `unavailable` 标记逻辑

* 同一 session 重复写入时去重与覆盖规则

* `Claude sessions-index.json + jsonl + subagents` 合并结果正确

### 集成测试

* 空 DB 首次启动，全量索引后 `list_sessions` 返回结果与当前 `readers::list_all()` 一致

* 修改一个 Codex `jsonl` 后，仅对应 session 更新

* 删除 Claude 主文件但保留 index/subagents 时，`available` / `archived` / `subagents` 状态正确

* watcher 在连续写文件时不会写入损坏状态，最终结果一致

### 前端验证

* 首次启动可直接显示 DB 中已有列表

* 后台索引完成后列表自动刷新

* 文件新增、更新、删除后，无需重启即可看到变化

* 详情页在文件被 agent 删除时，仍保留现有错误提示行为

## Assumptions

* 第一版目标是解决“启动全量扫盘慢”和“运行期间变更不同步”，不是做全文检索

* 消息正文仍以文件为准，数据库只作为索引缓存

* 第一版只正式落地 SQLite，本地数据库文件放在 Tauri app data 目录

* PostgreSQL 只做接口预留，不在第一版实现同步、鉴权、离线重试、冲突处理

* 当 watcher 事件不可靠或漏事件时，允许提供一个“手动重建索引”命令作为兜底

* 若未来要接远程 PG，推荐架构是“本地 SQLite 主读写 + 异步上报 PG”，而不是直接用 PG 取代本地缓存
