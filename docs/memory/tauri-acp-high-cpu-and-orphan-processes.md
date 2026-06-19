# Tauri Bundled App ACP High CPU And Orphan Processes

## Summary

这次问题发生在 Sessio 的 bundled macOS app 中：只要存在 ACP 会话，`sessio` 主进程就可能持续占用 `150%` 到 `360%+` CPU。最初怀疑过 ChatPage、WKWebView、session snapshot 推送频率，以及 shell wrapper，但这些都不是主因。

最终确认的问题在 Rust 后端的 ACP stdio transport 路径，而不是前端渲染层。旧实现依赖 `agent-client-protocol` crate 内建的 `AcpAgent::connect_to` + `async-process` 子进程管道处理。在 bundled app 的运行环境下，这条路径会进入高频热轮询，导致多个 tokio worker 长时间 busy-loop。

这次修复同时暴露了另一个桌面应用常见问题：子进程树如果只清理 direct child、不清理整个 process group，很容易留下孤儿 `npm/node/codex-acp` 进程。

## Symptoms

现象有几个非常稳定：

1. `Sessio.app` bundled 版本中，只要有 active ACP session，主进程 CPU 就明显偏高。
2. `WebContent` 进程虽然也有占用，但不是主要热点。
3. 同一套逻辑在本地 dev 模式下不明显，甚至基本不复现。
4. 结束会话或清理异常进程后，CPU 会显著下降。

这类现象很容易把排查方向带偏到 WebView、React render、message flood、节流参数上，但这次不是。

## Wrong Hypotheses

### 1. 前端 ChatPage 或 WKWebView 重渲染导致高 CPU

这个判断不成立。采样显示：

* 高 CPU 主要烧在 Rust 主进程，而不是 `WebContent`。
* `WebContent` 主线程多数时间在 `mach_msg` 空等，不符合持续重渲染打满的模式。
* 前端节流只会影响 UI 更新频率，不会解释 tokio worker 被持续占满。

结论：前端可能有自己的性能成本，但不是这次事故的主要原因。

### 2. session snapshot 频繁推送到前端导致 CPU 高

这个判断也不成立。即使 snapshot 推送偏频繁，它最多解释 UI 和 IPC 压力，解释不了采样里大量时间都落在后端 transport 的 `poll` 路径上。

更关键的是，CPU 热点明确出现在 ACP transport 的 futures 轮询链路中，而不是 session snapshot 构建或 Tauri event emit 上。

### 3. shell wrapper 是首要原因

这个判断只覆盖了一部分风险，但不是根因。

确实，shell wrapper 会带来更多层级的进程树、cwd 不透明、信号传播不稳定等问题。但把 wrapper 去掉之后，真正解决 CPU 的关键并不是“少套一层 shell”本身，而是完全绕开旧的 ACP stdio transport 路径，改为显式控制子进程和字节流。

## Evidence Trail

这次有效的证据链来自系统级采样和进程观察，而不是代码直觉。

### CPU 分布

* Rust 主进程一度烧到约 `360%` CPU。
* `WebContent` 只有约 `33%`，量级明显不是同一个级别。
* 增量 CPU 观测能看到 6 个 `tokio-rt-worker` 线程各自占用约 `55%`，总和接近主进程 CPU。

### 采样热点

热点落点集中在一条非常典型的异步热轮询路径：

* `tokio worker::run_task`
* `app_lib::agents::runtime::acp_transport::spawn_session::{closure}`
* `run_session`
* futures `select` / `poll_fn`
* `mpsc::Rx::pop`
* `AtomicWaker::register`

这说明 worker 并没有在正常 `park`，而是在某个 future 持续 `Ready` 或反复自唤醒的条件下空转。

### 孤儿进程

系统里还能观察到多条 `npm exec -> node -> codex-acp` 链条残留在 `PPID=1` 下。这不是高 CPU 的唯一原因，但说明会话退出路径和子进程树清理确实不完整。

## Root Cause

根因是旧 ACP transport 路径对运行环境敏感，并且 bundled app 更容易触发它的问题。

旧实现里，live session 和 probe 都通过 `agent-client-protocol` 的内建 transport 启动 ACP 子进程。那条路径内部使用 `async-process` 和它自己的 stdio/transport actor 组合。我们观察到 bundled app 环境下，这条链路会落入持续 poll 的 busy-loop。

从现象上看，它符合这类模式：

* 底层 stdio/EOF 状态变化后，某个分支 future 每次 poll 都能立刻返回 `Ready`，或者反复触发 `AtomicWaker`。
* 上层 `select` 持续重新 poll，同一个 tokio worker 无法回到空闲状态。
* 每多一个命中的 session，就多一份长期 CPU 占用。

这里最重要的结论不是“crate 某一行源码一定有 bug”，而是：

* 旧 transport 路径是黑盒较深的第三方实现。
* 它在 dev 和 bundle 两种运行环境下行为不一致。
* 桌面 app 里只要 transport 对运行环境敏感，就不应该把 session 生命周期完全托管给这类黑盒。

## Why Bundle Reproduced But Dev Mostly Did Not

这是这次问题最容易误判的点。

代码路径并不是 “dev 走 A，bundle 走 B”。两边原本都在用同一套 ACP transport 逻辑。区别在运行时上下文：

* bundled `sessio` 的 `cwd` 是 `/`
* dev 模式下的 `cwd` 更接近仓库 `src-tauri`
* dev 进程通常挂在 Tauri CLI/终端环境下
* bundled app 更接近 Finder / `launchd` 启动语境
* bundled app 的 stdio、父进程关系、环境变量和信号传播条件都不同

这说明问题不是“dev 没有 wrapper”或者“bundle 多了一层 wrapper”这么简单，而是旧 transport 路径对进程启动环境、stdio 状态、父子关系或 EOF 行为有敏感性。dev 没明显触发，不等于没有风险，只是更难暴露。

对未来项目的经验是：桌面 app 的 bundle 运行环境不是 dev 的近似值，而是另一个需要单独验证的目标平台。

## Fixes Applied

### 1. Live ACP session 不再使用 crate 内建 stdio transport

当前 live runtime session 改为在 Sessio 自己的运行时里显式启动：

* 使用 `tokio::process::Command`
* 显式设置 `current_dir(workspace_path)`
* 用 `shell_words::split` 解析命令
* 用 `ByteStreams` 把 tokio stdin/stdout 接到 ACP client

关键代码在 [acp_transport.rs](/Users/alex/Work/cloudgeek/sessio/src-tauri/src/agents/runtime/acp_transport.rs) 的 `spawn_acp_transport(...)` 和 `run_session(...)`。

这样做的价值有两个：

* 我们不再依赖第三方 transport 如何创建和驱动子进程。
* cwd、stdio 和 child lifecycle 都变成显式、可控、可测试的实现。

### 2. Probe 路径复用同一套 transport

这次不仅改了 live session，也把 runtime metadata probe 改成走同一套自定义 transport，而不是继续保留旧黑盒路径。

关键代码在：

* [acp_transport.rs](/Users/alex/Work/cloudgeek/sessio/src-tauri/src/agents/runtime/acp_transport.rs) 的 `probe_capabilities(...)` 和 `probe_initialize_response(...)`
* [metadata.rs](/Users/alex/Work/cloudgeek/sessio/src-tauri/src/agents/runtime/metadata.rs) 的 `detect_capabilities_with_initialize_only(...)`

这是一个重要经验：不要让“运行时主链路”和“probe/诊断链路”走两套不同的进程管理模型，否则问题会在冷启动、探测、运行态之间来回漂移。

### 3. Unix 下按 process group 清理整棵子进程树

为了避免 `npm exec -> node -> codex-acp` 这类链条在父进程退出后残留，现在在 Unix 下：

* 子进程启动时通过 `setpgid(0, 0)` 进入独立 process group
* `TokioChildGuard` drop 时优先对整个 process group 发 `SIGKILL`
* 非 Unix 环境仍退回 direct child `start_kill()`

关键代码也在 [acp_transport.rs](/Users/alex/Work/cloudgeek/sessio/src-tauri/src/agents/runtime/acp_transport.rs)：

* `configure_child_process_group(...)`
* `TokioChildGuard::drop`

这一步不是装饰性修复，而是桌面 app 中非常实际的资源卫生要求。只杀 direct child 不足以回收经 shell、npm、node 再派生出来的真实工作进程。

### 4. Astra delegated task 继续走 bounded cleanup

Astra 的 delegated runtime task 结束后，当前逻辑会走 bounded cleanup，再 dispose session，而不是把 live runtime 长期保留。

相关位置：

* [manager.rs](/Users/alex/Work/cloudgeek/sessio/src-tauri/src/agents/runtime/manager.rs) 的 `cleanup_session_bounded(...)`
* [runtime_agent_backend.rs](/Users/alex/Work/cloudgeek/sessio/src-tauri/src/astra/runtime_agent_backend.rs)
* [mod.rs](/Users/alex/Work/cloudgeek/sessio/src-tauri/src/astra/mod.rs) 的 `finish_delegated_task(...)`

这部分本身不是本次高 CPU 的根因，但它和“孤儿进程是否持续累积”直接相关。

## Validation

修复后，做过几类验证：

1. Rust 测试通过：
   * `cargo test acp_transport --manifest-path src-tauri/Cargo.toml`
   * `cargo test runtime_metadata --manifest-path src-tauri/Cargo.toml`
   * `cargo test spawned_unix_child_uses_its_own_process_group --manifest-path src-tauri/Cargo.toml -- --nocapture`
2. release 构建通过：
   * `cargo build --release --manifest-path src-tauri/Cargo.toml`
3. 运行态观察：
   * bundled `sessio` 主进程从 `150%+` / `360%+` 降到低个位数
   * `WebContent` 保持在更符合预期的占用水平
4. 进程卫生：
   * 系统中已有的 orphan `codex-acp` 树被清理
   * 未发现额外的 orphan `claude` 进程残留

这说明修复不是单纯“降低了一点频率”，而是把主热点从后端 transport busy-loop 上拿掉了。

## Lessons For Future Tauri Projects

### 1. 不要把“高 CPU + WebView 桌面 app”默认归因到前端

Tauri、Electron、WKWebView 项目里，前端天然更显眼，所以排查很容易先落到 React render、message flood、动画、虚拟列表上。但系统采样如果已经显示 CPU 烧在 Rust/Node/worker 线程，就应该立刻转向后端任务循环、stdio transport、IPC actor 和 timer/poll 逻辑。

### 2. bundle 必须被当成独立目标环境

桌面 app 的 dev 和 bundle 在这些维度上都可能不同：

* cwd
* stdio 是否绑定 TTY
* 父进程是谁
* 环境变量来源
* 信号传播
* sandbox / launch context

凡是和子进程、stdio、watcher、filesystem、shell、network bootstrap 相关的逻辑，都不能只在 dev 里验证一次就认为安全。

### 3. 子进程管理要显式，不要把生命周期藏在黑盒 transport 里

如果核心业务依赖外部 agent、CLI、语言服务器、索引器或 worker 进程，建议优先采用这种模式：

* 自己 `spawn`
* 自己设置 cwd
* 自己接管 stdin/stdout/stderr
* 自己定义退出和清理策略
* 自己给进程树做 group/Job Object 级别回收

第三方 transport 适合快速接入，不适合承载“长期存在、可能跨环境差异、资源泄漏代价高”的桌面 runtime 主链路。

### 4. 对 shell wrapper 保持谨慎

shell wrapper 不是原罪，但它会放大几个问题：

* direct child 不再等于真实 worker
* cwd 和 quoting 更难推断
* 退出码和信号传播更不透明
* orphan 进程更容易出现

能直接 exec 二进制或明确 argv 时，优先不用 shell。必须支持命令字符串时，也要在入口尽快把它解析成 argv，再进入显式的进程管理路径。

### 5. 会话结束语义必须覆盖“异常退出”和“父进程重启”

桌面 agent 类项目里，不能只处理“用户主动点关闭”的 happy path，还要显式考虑：

* 子进程自己崩溃
* shell/npm/node 中间层提前退出
* app 自身热重启
* 文件改动触发 dev 重载
* 页面关掉但后台 session 仍在

如果没有统一 cleanup contract，孤儿进程和空转 worker 最终一定会出现。

### 6. 资源卫生要有系统级验证手段

这次真正有价值的工具不是前端日志，而是：

* `ps`
* `sample`
* 线程级 CPU 观察
* orphan 进程扫描
* 真实 bundle 运行态检查

以后遇到类似问题，先回答这几个问题：

1. CPU 烧在哪个进程、哪个线程？
2. 线程是在算业务，还是在 poll/lock/wake 空转？
3. 子进程树是否按预期退出？
4. dev 和 bundle 的 cwd、stdio、父进程关系是否一致？

## Recommended Guardrails

未来在 Tauri 项目里，只要引入长期运行的外部进程、agent、LSP、CLI worker，建议默认加上这些 guardrail：

1. 所有 runtime child process 都显式设置工作目录，不依赖 app 当前 cwd。
2. 所有 runtime child process 都走统一的 spawn 封装，不允许部分路径绕过。
3. 统一记录 child pid、argv、cwd、spawn time、exit status。
4. Unix 下默认启用 process group；Windows 下要有等价的 Job Object 或 tree cleanup 策略。
5. child drop/dispose 必须能杀整棵进程树，而不是只杀 direct child。
6. dev、test、bundle 三个环境都要做最小回归验证，至少覆盖一次 active session。
7. 对 transport/IPC loop 增加 watchdog 指标，例如 idle loop 次数、连续 wake 次数或异常高频 poll 计数。
8. probe、healthcheck、runtime session 不要用不同的子进程管理模型。
9. cleanup 要覆盖正常完成、错误退出、取消、超时和宿主重启。
10. 对 orphan process 增加可操作的巡检命令或诊断入口。

## Suggested Debugging Playbook

如果以后在别的 Tauri 项目中再遇到“bundle 高 CPU，但 dev 正常”的情况，可以按这个顺序排查：

1. 先用系统采样确认热点是在前端 WebContent、宿主后端，还是外部子进程。
2. 如果热点在宿主后端，先看 async task、channel、select、poll_fn、watch/mpsc、timer loop。
3. 如果项目依赖外部 CLI/agent，检查是不是 transport/stdio/EOF/child exit 路径在空转。
4. 对比 dev 和 bundle 的 cwd、stdio、env、parent process、spawn argv。
5. 检查进程树回收是否覆盖 shell/npm/node 等中间层。
6. 如果主链路依赖第三方黑盒 transport，优先评估是否改成自管 spawn + streams。

## Bottom Line

这次事故的核心经验不是“某个 crate 有 bug”，而是：

* 桌面 app 的 bundle 环境会放大进程管理问题。
* 只要有长期外部子进程，transport 和 cleanup 就应该被视为一等系统设计问题。
* 对高 CPU 的判断必须先看系统级证据，再决定是前端问题、后端问题，还是子进程问题。

对未来的 Tauri 项目来说，最有效的预防方式不是多加节流，而是从一开始就把子进程启动、transport、退出和 orphan cleanup 做成显式、统一、可验证的基础设施。
