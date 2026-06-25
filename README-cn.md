<p align="center">
  <img src="assets/logo.jpg" alt="Sessio logo" width="960" />
</p>

<h1 align="center">Sessio</h1>

<p align="center">面向 coding agent 的桌面工作台：浏览本地会话历史、与 agent 实时对话、编排多智能体协作。</p>

<p align="center">
  <a href="./README-cn.md">中文</a> · <a href="./README.md">English</a>
</p>

## 功能特性

### 会话浏览

- 聚合 `Codex`、`Claude Code`、`Gemini`、`OpenCode` 以及外部 `Pi` 的本地会话
- 使用 `SQLite` 建立本地索引，避免每次启动都全量扫盘
- 监听文件变化（辅以定时轮询）并自动刷新列表
- 侧边栏按项目分组浏览，支持未读标记与实时运行状态指示
- 消息时间线支持 markdown、KaTeX 公式、代码高亮和文件编辑 diff 渲染
- 支持 Claude subagents 展示、会话重命名和删除

### 实时对话

- 通过 [Agent Client Protocol (ACP)](https://agentclientprotocol.com) 在应用内直接发起和续写 agent 会话
- 流式展示文本、推理过程和工具调用，可在对话中直接响应权限请求
- 每个会话可单独选择模型、推理强度和权限模式，支持图片和文件附件
- 跨 agent 续写：把会话 fork 给另一个 agent，自动携带上下文继续

### Channels — 从 IM 调用本地 agent

- 把外部聊天平台接入桌面端同一套 Sessio runtime，IM 消息会变成 agent prompt
- 支持 Telegram Bot、Discord Bot、Lark / 飞书长连接，以及 WeChat iLink Bot
- 每个 channel 可以独立配置默认 agent、模型、推理强度和工作区
- 持久化 chat 与 agent session 的绑定，应用重启后可继续同一条 IM 会话
- 在平台支持时可收发文本、图片和文件，agent 输出里的本地文件会上传回 IM
- Telegram 支持 slash command 和 inline menu：`/new`、`/agent`、`/model`、`/effort`、`/workspace`、`/cancel`、`/end`
- agent 的权限请求会转成 IM 内的批准 / 拒绝交互（平台支持时）
- 面向本地桌面使用：Telegram、Discord、Lark 都是主动向外连接，不需要公网 HTTP 服务

### Threads — 多智能体协作

- 四种 thread 类型：`Workflow`（阶段化流程模板）、`Teamwork`（项目助手协作）、`Brainstorm`（两个及以上参与者）、`Debate`（恰好两个参与者）
- 多会话聊天时间线，展示每条 lane 的状态、轮次和编排记录
- Workflow thread 提供阶段跟踪：阶段状态、总结 / 产出，以及按阶段的 issue 管理
- 内置和自定义流程模板，支持拖拽编辑阶段

### Astra 编排器

- Rust 原生、进程内的编排器，负责为 thread 规划任务并分发给各 agent
- 计划轮次与任务支持依赖感知的波次分发、失败重试，任务产出写入 `<project>/.sessio/astra`
- 编排所用的 agent / 模型 / 推理强度 / 权限模式均可配置，默认使用 `Codex` 作为规划 backend

### 其他

- 自定义助手（底层 agent + 模型 + 系统提示词 + 权限模式），可全局或按项目管理
- Project memory：基于会话构建可检索的记忆记录（`qmd` 后端），支持续写溯源（`covered-by` / `base`）
- CLI 模式，提供 `sessions`、`thread`、`stage`、`config`、`memory` 命令组
- 托盘菜单快速打开最近的会话和 thread
- 应用内更新（Tauri updater 产物），并以 GitHub Releases 检查作为兜底
- 中英文界面、浅色 / 深色 / 跟随系统主题、HTTP(S) 代理设置

## 支持的数据来源

Sessio 直接读取本机已有的会话文件，不依赖云端服务。

默认会扫描这些目录：

- Codex
  - `~/.codex/sessions`
  - `~/.codex/archived_sessions`
- Claude Code
  - `~/.claude/projects`
- Gemini
  - `~/.gemini/tmp`
  - `~/.gemini/projects.json`
- Pi
  - `~/.pi/agent/sessions`

应用数据存放目录：

- release 正式版：`~/.sessio`
- debug / 本地开发版：`~/.sessio-dev`

例如：

- `~/.sessio/db-data/sessio-index.db` — SQLite 索引
- `~/.sessio/config.toml` — memory / 索引 / 代理 / 调试配置
- `~/.sessio/im-bridge.yaml` — Telegram / Discord / Lark / WeChat 的 Channels 配置
- `~/.sessio/bin/sessio` — 启动时创建的 CLI 软链接

## Agent 运行时

实时对话会以本地子进程方式启动 agent。ACP agent 使用 ACP adapter，Pi 使用 RPC 模式。默认命令：

- Codex：`npx -y @agentclientprotocol/codex-acp@latest`
- Claude Code：`npx -y @zed-industries/claude-code-acp@latest`
- OpenCode：`opencode acp`
- Pi（默认关闭）：`pi --mode rpc`

在设置 → Agents 中可以启用 / 禁用各 agent，并编辑模型目录、默认模型、推理强度和权限模式。Astra 编排器使用的 agent 也在同一设置区域单独配置。

Channels 可在设置 → Workflows → Channels 中配置。Sessio 会把各平台默认值和 workspace allowlist 写入当前应用目录下的 `im-bridge.yaml`，活跃 chat 绑定则保存在本地 SQLite 索引中。由于 channel 可以驱动能运行本机工具的 agent，启用前请限制允许的用户 / 聊天和工作区。

## 技术栈

- 前端：`React 19` + `TypeScript` + `Vite` + `Tailwind CSS`
- 桌面壳：`Tauri v2`
- 后端：`Rust`（edition 2021，`agent-client-protocol`）
- 存储：`SQLite (rusqlite bundled)`

后端大致分为几个模块：

- `src-tauri/src/agents/sources`：解析不同 agent 的原始会话文件
- `src-tauri/src/agents/runtime`：通过 ACP 运行实时 agent 会话
- `src-tauri/src/astra`：多智能体编排器
- `src-tauri/src/im_bridge`：Channels 集成和 IM 到 runtime 的路由
- `src-tauri/src/store`：本地索引存储
- `src-tauri/src/indexer`：全量重建与增量更新
- `src-tauri/src/watch`：文件监听
- `src-tauri/src/polling.rs`：轮询补偿刷新
- `src-tauri/src/memory`：project memory 流水线（`qmd` 后端）
- `src-tauri/src/turns.rs`：把原始事件归一化为可渲染的 turn
- `src-tauri/src/cli.rs`：`sessio` CLI

## 开发环境

建议版本：

- `Node.js 24.x`
- `pnpm 11.1.0`
- `Rust 1.95+`

安装依赖：

```bash
pnpm install
```

启动前端开发服务器：

```bash
pnpm dev
```

启动 Tauri 桌面开发模式：

```bash
pnpm tauri dev
```

类型检查：

```bash
pnpm typecheck
```

运行测试：

```bash
pnpm test
```

类型检查加测试：

```bash
pnpm check
```

构建前端：

```bash
pnpm build
```

构建桌面应用但不打包安装器：

```bash
pnpm ci:build
```

构建发布包：

```bash
pnpm bundle
```

## 平台说明

仓库当前已包含多平台发布配置，GitHub Actions 会产出安装包和 updater 产物：

- macOS 通用 `.dmg`，以及签名后的 `.app.tar.gz` updater 包
- Linux `x86_64` `.deb` / `.rpm` 及 updater 签名
- Linux `arm64` `.deb` / `.rpm` 及 updater 签名
- Windows `x86_64` NSIS 安装器及 updater 签名

发布 workflow 需要在 GitHub Secrets 中配置 Tauri updater 私钥
`TAURI_SIGNING_PRIVATE_KEY`。如果私钥没有密码，
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 可以不配置。

Linux 构建通常需要先安装 Tauri/WebKitGTK 依赖，例如：

- `libwebkit2gtk-4.1-dev`
- `libgtk-3-dev`
- `libsoup-3.0-dev`
- `libjavascriptcoregtk-4.1-dev`
- `libssl-dev`

## 使用方式

启动后，Sessio 会在后台建立索引并展示会话列表。

你可以：

- 在侧边栏按项目浏览会话和 thread
- 打开详情页查看消息、工具调用和 diff
- 用任意已启用的 agent 发起新对话，或续写已有会话
- 把会话 fork 给另一个 agent，在那边继续上下文
- 创建 thread（workflow / teamwork / brainstorm / debate），交给 Astra 编排执行
- 在 thread 页面跟踪 workflow 阶段和 issue
- 启用 Channels，从 Telegram、Discord、Lark 或 WeChat 与本地 agent 对话
- 从托盘菜单快速进入最近的会话和 thread

Sessio 也可以作为 CLI 运行，例如：

```bash
sessio sessions list --json
sessio sessions messages --agent codex --session-id <id> --json
sessio thread list --json
sessio stage list --thread-id <id> --json
sessio memory search --project "$PWD" <query> --json
sessio memory resolve --record-id <id> --json
```

命令组一览：

- `sessions` — `list`、`messages`
- `thread` — `list`、`show`
- `stage` — `list`、`show`、`set-status`、`update`，以及 `issue add | list | set`
- `config` — `show`、`memory set`
- `memory` — `status`、`sync`、`build`、`search`、`resolve`、`covered-by`、`base`、`jobs`

当原始会话文件被工具清理后，Sessio 仍会尽量保留索引元数据；如果正文文件已经不存在，详情页会提示该会话内容不可再读取。

## 项目结构

```text
.
├── src/                  # React 前端
├── src-tauri/            # Tauri + Rust 后端
├── docs/                 # 设计与实现文档
├── scripts/              # 发布辅助脚本
├── test/                 # 前端单元测试（vitest）
├── package.json
└── README-cn.md
```

## 发布

本地版本发布脚本：

```bash
pnpm release -- 0.5.0
# 或发布 beta / prerelease tag：
pnpm release -- 0.5.0-beta.1
```

或直接执行：

```bash
./scripts/release.sh 0.5.0
```

这个脚本会：

- 更新 `package.json`
- 更新 `src-tauri/Cargo.toml`
- 更新 `src-tauri/tauri.conf.json`
- 刷新 `src-tauri/Cargo.lock`
- 创建本地 release commit 和 tag

推送 tag 后会触发 GitHub Actions 发布流程。
带 prerelease 后缀的 tag（例如 `v0.5.0-beta.1`）会发布为 GitHub prerelease，并且不会被标记为 latest release。

## 已知边界

- 目前只索引会话元数据，详情内容仍按需读取原始文件
- 不同 agent 的原始日志格式差异较大，兼容逻辑依赖当前本地文件结构
- 如果第三方工具未来调整目录结构或日志格式，对应的解析器可能需要同步更新
- Codex / Claude Code / Gemini 的实时运行时通过 `npx` 拉取，需要本机具备对应 agent 的账号 / API 访问能力

## License

仓库内暂未声明许可证；如需开源分发，建议补充 `LICENSE` 文件。
