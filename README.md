# claudie

中文 | [English](README.en.md)

`claudie` 是一个为 Claude Code 设计的轻量桌面宠物。Windows 版本使用 Rust + Win32/GDI+ 原生窗口实现，运行时主要由一个 UI 线程、一个同步 `std::net::TcpListener` hook server，以及一个本地 Anthropic Messages 到 OpenAI Chat Completions 的兼容代理组成。

项目刻意避免 Electron、WebView、async runtime 和 Web 框架。宠物资源使用轻量 GIF 动画目录，每种 mood 对应一个 GIF 文件。

## 致谢

claudie 受 [rullerzhou-afk/clawd-on-desk](https://github.com/rullerzhou-afk/clawd-on-desk) 和 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) 启发。

## 功能

- **Hook 事件驱动**：接收 Claude Code HTTP hooks，并根据事件切换宠物状态。

  | 事件 | 行为 |
  |------|------|
  | `SessionStart` / `SessionResume` | 回到空闲待命 |
  | `UserPromptSubmit` | 思考中 |
  | `PreToolUse` | 开始执行工具，例如 Write -> typing、Bash -> building、Read/Grep/Glob -> search |
  | `PostToolUse` | 工具完成 |
  | `PostToolBatch` | 一批工具完成，并刷新配额快照 |
  | `PostToolUseFailure` / `StopFailure` / `PermissionDenied` | 错误状态 |
  | `SubagentStart` / `TaskCreated` | 子任务进行中 |
  | `SubagentStop` / `TaskCompleted` | 子任务完成 |
  | `PreCompact` | 上下文压缩中 |
  | `PostCompact` | 压缩完成 |
  | `Notification` / `Elicitation` | 通知提示 |
  | `WorktreeCreate` | 创建工作目录 |
  | `Stop` | 任务结束 |
  | `SessionEnd` | 会话结束，清除所有待处理交互 |

- **权限请求**：通过 `PermissionRequest` hook 接管权限请求，在宠物窗口中显示 Allow / Always Allow / Deny 控件。
- **选择卡片**：支持 `PreToolUse` 中的 `AskUserQuestion` 和 `ExitPlanMode` 交互，显示选项列表以及 Submit / Cancel 控件。
- **快捷键**：
  - `Ctrl+Shift+Y`：允许当前权限请求，或提交当前选择。
  - `Ctrl+Shift+N`：拒绝当前权限请求，或取消当前选择。
- **番茄钟**：内置 Pomodoro，支持 Start / Stop / Pause / Resume / Skip，阶段结束时弹出通知。
- **宠物互动**：短按左键会播放互动动画，按住并移动仍可拖动窗口；番茄钟专注时可使用专属 `pomodoro` 动画。
- **空闲睡眠**：长时间无活动后自动进入睡眠状态，有新活动时唤醒。
- **宠物缩放**：可调整宠物窗口大小。
- **窗口位置记忆**：退出时保存窗口位置，下次启动自动恢复。
- **Mood -> GIF 映射**：每种情绪状态都可配置对应 GIF 文件。
- **Settings 面板**：提供 Basic、Pomodoro、LLM Profiles、Stats 四个标签页，使用统一的 Slint 原生主题。
- **LLM Profiles**：保存 LLM provider/profile，将当前 profile 写入 Claude Code settings，并为 OpenAI 代理配置额外请求体字段；右键菜单可快速切换已保存 profile。
- **会话小账本**：在本地按天记录 prompts、工具类型、权限/选择次数、错误、番茄钟完成数和 token 用量，并在 Stats 页以柱状图展示今日与最近 7 天。
- **OpenAI 兼容代理**：把 Claude Code 的 Anthropic Messages 请求转换到 OpenAI Chat Completions API，支持工具调用格式转换、并行工具调用控制、上下文压缩、历史总结和能力缓存。
- **跨平台**：Windows 提供完整桌面 UI；macOS / Linux 当前仅运行 headless hook 与 proxy 服务，没有桌面交互 UI。

## 快速开始

开发运行：

```powershell
cargo run --release
```

正常启动会确保 Claude Code hooks 指向当前 claudie 端口；Windows UI 退出时会清理 claudie 管理的 hooks。下面的安装/卸载命令主要用于手动管理或打包流程。

安装 Claude Code hooks：

```powershell
cargo run --release -- --install-claude-hooks
```

卸载 Claude Code hooks：

```powershell
cargo run --release -- --uninstall-claude-hooks
```

打印可手动合并的 Claude Code settings 片段：

```powershell
cargo run --release -- --print-claude-settings
```

指定 hook 端口：

```powershell
cargo run --release -- --port 17387
```

静默模式，抑制安装或卸载时的系统通知弹窗：

```powershell
cargo run --release -- --install-claude-hooks --quiet
```

## OpenAI 兼容 API 代理

claudie 启动时会同时监听本地代理地址：

```text
http://127.0.0.1:17388
```

在 Settings -> LLM Profiles 中新增或编辑一个 profile：

- `Base URL` 填 OpenAI 兼容的 chat completions 地址，例如 `https://example.com/v1/chat/completions`。
- `API key` 填上游 OpenAI 兼容服务的 key。
- `Model` 填该服务支持的模型名。
- `OpenAI body` 可选填额外请求体字段，支持 JSON object 或逐行 `key=value` / `key: value`，例如 `{"reasoning_effort":"xhigh"}` 或 `model_reasoning_effort = "xhigh"`。

点击 `Use` 后，claudie 会把 Claude Code 的 `ANTHROPIC_BASE_URL` 写成本地代理地址；上游请求地址和 key 只保存在 claudie 的 profile 中。`Base URL` 只要包含 `/chat/completions` 就会自动启用代理；如果填的是上游根地址，可在 `Extra env` 中加入：

```text
CLAUDIE_API_FORMAT=openai
```

代理当前实现了 `POST /v1/messages`、`POST /v1/messages/count_tokens` 和 `GET /v1/models`，并支持在 Anthropic / OpenAI 工具调用格式之间转换。`OpenAI body` 会合并进转发到上游的 chat completions 请求，但不能覆盖 claudie 管理的 `messages` 和 `stream` 字段。

当请求包含工具时，代理默认向上游设置 `parallel_tool_calls=true`，让模型可以批量发起相互独立的工具调用，例如一次读取多个文件、一次 stage 多个路径。若某些较旧或较小的上游模型在并行工具调用下表现异常，可在 `OpenAI body` 中显式设置 `{"parallel_tool_calls": false}` 关闭并行。代理也会注入一段很短的兼容提示，帮助 OpenAI 格式模型理解 tool result；如果不需要，可通过 `CLAUDIE_PROXY_COMPAT_PROMPT=0` 关闭。

OpenAI 代理默认启用上下文优化。claudie 会在转发请求前压缩超长工具结果和普通文本；当估算输入超过阈值时，会保留最近消息并对更早的对话分块总结。每个消息块独立总结并缓存到 `%USERPROFILE%\.claudie\proxy_cache\` 下的 `summaries/`、`chunks/` 和 `capabilities/` 目录。缓存只保存总结文本或能力探测结果，不保存 API key 或完整原始请求体。

可在 profile 的 `Extra env` 中调整或关闭该行为：

```text
CLAUDIE_PROXY_OPTIMIZE=0
CLAUDIE_PROXY_SUMMARY_MODE=local
CLAUDIE_PROXY_SUMMARY_THRESHOLD=24000
CLAUDIE_PROXY_KEEP_RECENT_MESSAGES=12
CLAUDIE_PROXY_KEEP_RECENT_TOKENS=10000
CLAUDIE_PROXY_TOOL_RESULT_LIMIT=3000
CLAUDIE_PROXY_TEXT_LIMIT=6000
CLAUDIE_PROXY_MAX_OUTPUT_TOKENS=4096
CLAUDIE_PROXY_LOCAL_SUMMARY_TOKENS=2000
CLAUDIE_PROXY_CACHE_MAX_MB=10
CLAUDIE_PROXY_SUMMARY_CACHE_TTL_HOURS=168
CLAUDIE_PROXY_SUMMARY_CACHE_MAX_ENTRIES=200
CLAUDIE_PROXY_CHUNK_SUMMARY=1
CLAUDIE_PROXY_CHUNK_SIZE_MESSAGES=8
CLAUDIE_PROXY_CHUNK_CACHE_TTL_HOURS=168
CLAUDIE_PROXY_CHUNK_CACHE_MAX_ENTRIES=200
CLAUDIE_PROXY_CAPABILITY_CACHE_TTL_HOURS=168
CLAUDIE_PROXY_CAPABILITY_CACHE_MAX_ENTRIES=200
```

默认使用本地抽取式总结，也就是 `CLAUDIE_PROXY_SUMMARY_MODE=local`，不会为了总结再调用昂贵的上游模型。若希望使用模型生成总结，可设置 `CLAUDIE_PROXY_SUMMARY_MODE=model`；如果模型总结失败，claudie 仍会转发只经过长内容压缩的请求。设置 `CLAUDIE_PROXY_MAX_OUTPUT_TOKENS=0` 可关闭输出 token 上限。

分块总结默认开启，会把较早消息按 `CLAUDIE_PROXY_CHUNK_SIZE_MESSAGES` 条一组分块，逐块生成摘要并附加到压缩结果中。设置 `CLAUDIE_PROXY_CHUNK_SUMMARY=0` 可关闭分块，退回单块总结。`proxy_cache/` 目录和旧版 `proxy_summaries.json` 都可以安全删除，claudie 会在需要时重新生成。

## 项目结构

```text
src/
  main.rs                  CLI 参数、应用启动、hook/profile 初始化和平台入口
  config.rs                端口、尺寸、菜单 ID、颜色和定时常量
  globals.rs               进程级 OnceLock 全局句柄
  notifier.rs              平台通知 / 消息框封装
  util.rs                  参数解析、路径、文本截断和 UTF-16 辅助函数
  app/                     AppState、权限请求、选择请求、番茄钟等领域规则
    mod.rs                 AppState、PetMood、会话、配额、待处理交互、统计和 mood 衰减
    pomodoro.rs            轻量番茄钟状态与转换
    stats.rs               本地每日会话小账本、工具分类和 token 统计
  hooks/                   Claude Code hook server、事件语义、配额提取、settings 合并
    claude_settings.rs     hook settings 生成、安装、卸载与合并
    events.rs              Claude Code 事件处理、权限等待和选择响应
    quota.rs               token、模型、provider 与配额字段兼容提取
    server.rs              接收 POST /hook 的最小同步 HTTP server
  proxy.rs                 本地 Anthropic Messages -> OpenAI Chat Completions 代理
  proxy_optimizer.rs       长上下文压缩、分块历史总结和总结缓存
  settings/                用户设置、LLM profiles、Claude env 集成、JSON 存储 helper
    mod.rs                 持久化设置、profile 数据库、OpenAI body 解析和路径规范化
    storage.rs             BOM 兼容读取和 pretty JSON 写入
  ui/
    gif_animation.rs       GIF 加载、帧延迟读取、mood 转场和 GDI+ 绘制
    theme.rs               Settings 面板和权限/选择弹层的共享视觉 token
    window/                主宠物窗口
      mod.rs               窗口生命周期、热键、菜单、点击处理和位置持久化
      render.rs            HUD、宠物绘制、权限弹层和选择卡片绘制
    slint_views.rs         Settings 窗口和 Prompt 弹窗的 Slint 组件声明
    settings_panel/        Slint Settings 面板生命周期、回调和控制器逻辑
      controller.rs        SettingsController 共享状态与同步 helper
      controller/          Basic、Pomodoro、LLM Profiles、Stats 分区行为
    prompt_popup.rs        Slint 权限/选择弹窗快照和回调
    window_icon.rs         Slint/Winit/Win32 辅助窗口图标桥接
```

其它目录：

- `assets/claudie/`：内置宠物 GIF 动画资源。
- `assets/icon.*`、`assets/claudie.manifest`：应用图标和 Windows manifest。
- `packaging/`：Windows/Unix 打包与安装脚本。

## 本地数据

- `%USERPROFILE%\.claudie\settings.json`：宠物资源目录、GIF 目录、动画映射、缩放、睡眠时间、窗口位置和番茄钟设置。
- `%USERPROFILE%\.claudie\llm_profiles.json`：LLM provider/profile 定义，包括 OpenAI 代理额外请求体字段。
- `%USERPROFILE%\.claudie\daily_stats.json`：每日会话小账本，只保存计数，包括工具分类、权限/选择、番茄钟完成数和 token 用量。
- `%USERPROFILE%\.claudie\proxy_summaries.json`：旧版单块总结缓存。
- `%USERPROFILE%\.claudie\proxy_cache\`：OpenAI 代理缓存目录，包含：
  - `summaries/`：单块总结缓存 JSON 文件。
  - `chunks/`：分块总结缓存 JSON 文件，每个消息块独立缓存。
  - `capabilities/`：上游模型工具历史兼容能力缓存。
- `%USERPROFILE%\.claude\settings.json`：Claude Code hook settings 和 claudie 管理的 LLM env。
- `%USERPROFILE%\.claude\settings.json.claudie.bak`：首次修改 Claude settings 前创建的一次性备份。

## 宠物资源

内置资源位于：

```text
assets/claudie/
  idle.gif
  thinking.gif
  typing.gif
  building.gif
  search.gif
  happy.gif
  error.gif
  sleeping.gif
  subagent.gif
  pomodoro.gif
  wave.gif
  stretch.gif
```

Settings 面板可以调整 GIF 目录和每个 mood 对应的文件名。替换美术资源时保持文件名映射一致即可。

## 维护边界

- `AppState` 是中央可变模型；长期状态和领域规则优先放在 `src/app/`。
- 每日会话小账本属于 `src/app/stats.rs`；hook 事件只负责在 `src/hooks/events.rs` 中调用统计记录，不要在 UI 层推导业务计数。
- Hook server 保持小而同步；HTTP 解析留在 `src/hooks/server.rs`，Claude event 语义放在 `src/hooks/events.rs`。
- 配额字段兼容逻辑集中在 `src/hooks/quota.rs`。
- 修改 Claude settings 时只合并 claudie 管理的 hook/env 字段，保留用户其它配置。
- 新增 JSON 状态文件时复用 `src/settings/storage.rs`。
- OpenAI 代理的请求/响应转换放在 `src/proxy.rs`；上下文优化、长文本压缩、分块历史总结和总结缓存放在 `src/proxy_optimizer.rs`。
- UI 线程不要做可能卡顿的网络或文件工作。
- 主窗口新增可视元素优先改 `src/ui/window/render.rs`；新增菜单、热键或鼠标交互优先放 `src/ui/window/mod.rs`。
- Settings 面板和权限/选择弹层的颜色、圆角、字体等共享视觉 token 放在 `src/ui/theme.rs`。
- Settings 面板新增字段时，Slint 组件声明放在 `src/ui/slint_views.rs`，回调接线放在 `src/ui/settings_panel/mod.rs`，保存/刷新行为放在 `src/ui/settings_panel/controller/`。

## 打包

Windows 安装包模板位于 `packaging/windows/claudie.iss`：

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
```

## 验证

提交前至少运行：

```powershell
cargo fmt
cargo check
```

修改 hook settings、配额提取、LLM profile、代理转换或纯领域规则时，也运行：

```powershell
cargo test
```

涉及 UI、hook、权限、settings 或代理行为时，再手动运行：

```powershell
cargo run --release
```

重点检查宠物窗口是否打开并恢复上次位置、右键菜单是否可用并能快速切换 LLM Profile、Basic/Pomodoro/LLM Profiles/Stats tabs 是否正常切换、左键点击是否播放互动动画且拖动仍可用、GIF 资源是否加载、`POST /hook` 是否更新状态、权限/选择卡片是否可交互、Stats 页柱状图是否不溢出，以及需要时本地 LLM 代理和 `OpenAI body` 转发是否工作。
