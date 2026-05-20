<p align="center">
  <img src="assets/logo.jpg" alt="Sessio logo" width="960" />
</p>

<h1 align="center">Sessio</h1>

<p align="center">用于管理本地多 agent 会话历史的桌面工具。</p>

<p align="center">
  <a href="./README-cn.md">中文</a> · <a href="./README.md">English</a>
</p>

## 功能特性

- 聚合 `Codex`、`Claude Code`、`Gemini` 的本地会话
- 使用 `SQLite` 建立本地索引，避免每次启动都全量扫盘
- 监听文件变化并自动刷新列表
- 按助手、按项目筛选会话
- 查看会话详情与消息时间线
- 支持 Claude subagents 展示
- 为原助手复制 `resume` 命令
- 生成跨助手续写命令，把一个助手的上下文接续到另一个助手
- 提供 CLI，可列出会话并操作项目记忆
- 支持构建、检索和回源 project memory record
- 托盘菜单快速打开最近会话
- 支持中英文界面、浅色 / 深色 / 跟随系统主题
- 发布版内置 GitHub 最新版本检查

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

索引数据库会写入：

- `~/.sessio/db-data/sessio-index.db`

## 技术栈

- 前端：`React 19` + `TypeScript` + `Vite` + `Tailwind CSS`
- 桌面壳：`Tauri v2`
- 后端：`Rust`
- 存储：`SQLite (rusqlite bundled)`

后端大致分为几个模块：

- `src-tauri/src/readers`：解析不同助手的原始会话文件
- `src-tauri/src/store`：本地索引存储
- `src-tauri/src/indexer`：全量重建与增量更新
- `src-tauri/src/watch`：文件监听
- `src-tauri/src/polling.rs`：轮询补偿刷新

## 开发环境

建议版本：

- `Node.js 24.x`
- `pnpm 11.1.0`
- `Rust 1.85+`

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

仓库当前已包含多平台发布配置，GitHub Actions 会产出：

- macOS 通用二进制
- Linux `x86_64`
- Linux `arm64`
- Windows `x86_64`

Linux 构建通常需要先安装 Tauri/WebKitGTK 依赖，例如：

- `libwebkit2gtk-4.1-dev`
- `libgtk-3-dev`
- `libsoup-3.0-dev`
- `libjavascriptcoregtk-4.1-dev`
- `libssl-dev`

## 使用方式

启动后，Sessio 会在后台建立索引并展示会话列表。

你可以：

- 在侧边栏按助手或项目浏览会话
- 打开详情页查看消息内容
- 对同一助手复制 `resume` 命令
- 对其他助手复制 `cross` 命令，把上下文迁移过去继续对话
- 从托盘菜单快速进入最近会话

Sessio 也可以作为 CLI 运行，例如：

```bash
sessio sessions list --json
sessio sessions messages --agent codex --session-id <id> --json
sessio memory search --project "$PWD" <query> --json
sessio memory resolve --record-id <id> --json
```

`memory` 命令组还包括 `build`、`search`、`resolve`、`base`、`covered-by`、`status`、`sync` 和 `jobs`。

当原始会话文件被工具清理后，Sessio 仍会尽量保留索引元数据；如果正文文件已经不存在，详情页会提示该会话内容不可再读取。

## 项目结构

```text
.
├── src/                  # React 前端
├── src-tauri/            # Tauri + Rust 后端
├── docs/                 # 设计与实现文档
├── scripts/              # 发布辅助脚本
├── package.json
└── README-cn.md
```

## 发布

本地版本发布脚本：

```bash
pnpm release -- 0.3.3
```

或直接执行：

```bash
./scripts/release.sh 0.3.3
```

这个脚本会：

- 更新 `package.json`
- 更新 `src-tauri/Cargo.toml`
- 更新 `src-tauri/tauri.conf.json`
- 刷新 `src-tauri/Cargo.lock`
- 创建本地 release commit 和 tag

推送 tag 后会触发 GitHub Actions 发布流程。

## 已知边界

- 目前只索引会话元数据，详情内容仍按需读取原始文件
- 不同助手的原始日志格式差异较大，兼容逻辑依赖当前本地文件结构
- 如果第三方工具未来调整目录结构或日志格式，reader 可能需要同步更新

## License

仓库内暂未声明许可证；如需开源分发，建议补充 `LICENSE` 文件。
