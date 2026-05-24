# claudie

中文 | [English](README.en.md)

`claudie` 是一个为 Claude Code 设计的轻量桌面宠物。Windows 版本使用 Rust + Win32/GDI+ 原生窗口实现，运行时主要是一条 UI 线程、一个同步 `std::net::TcpListener` hook server，以及一个本地 LLM 代理。

项目刻意不引入 Electron、WebView、async runtime 或 Web 框架。宠物资源采用轻量 GIF 动画目录，每个 mood 对应一个 GIF 文件。

## 致谢

claudie 受 [rullerzhou-afk/clawd-on-desk](https://github.com/rullerzhou-afk/clawd-on-desk) 和 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) 启发。

## 功能

- **Hook 事件驱动**：接收 Claude Code HTTP hooks，根据以下事件切换宠物状态：

  | 事件 | 行为 |
  |------|------|
  | `SessionStart` / `SessionResume` | 回到空闲待命 |
  | `UserPromptSubmit` | 思考中 |
  | `PreToolUse` | 开始执行工具（如 Write→typing、Bash→building） |
  | `PostToolUse` | 工具完成 |
  | `PostToolBatch` | 一批工具全部完成，触发配额快照 |
  | `PostToolUseFailure` / `StopFailure` / `PermissionDenied` | 错误状态 |
  | `SubagentStart` / `TaskCreated` | 子任务进行中 |
  | `SubagentStop` / `TaskCompleted` | 子任务完成 |
  | `PreCompact` | 上下文压缩中（thinking 态） |
  | `PostCompact` | 压缩完成 |
  | `Notification` / `Elicitation` | 通知提示 |
  | `WorktreeCreate` | 创建工作目录（building 态） |
  | `Stop` | 任务结束 |
  | `SessionEnd` | 会话结束，清除所有待处理交互 |

- **权限请求**：通过 `PermissionRequest` hook 接管权限请求，在宠物窗口里显示 Allow / Always Allow / Deny 按钮。
- **选择卡片**：支持 `PreToolUse` 中的 `AskUserQuestion` 和 `ExitPlanMode` 交互式选择卡片，显示选项列表 + Submit / Cancel 按钮。
- **快捷键**：
  - `Ctrl+Shift+Y`：允许当前权限请求 / 提交当前选择
  - `Ctrl+Shift+N`：拒绝当前权限请求 / 取消当前选择
- **番茄钟**：内置番茄钟（Pomodoro），支持 Start / Stop / Pause / Resume / Skip，结束时弹出通知。
- **空闲睡眠**：闲置一段时间后宠物自动进入睡眠状态，有活动时唤醒。
- **宠物缩放**：支持缩放宠物窗口大小。
- **窗口位置记忆**：退出时保存窗口位置，下次启动恢复。
- **Mood → GIF 映射**：每种情绪状态可配置对应的 GIF 文件。
- **Settings 面板**：包含 Basic、Pomodoro、LLM Profiles 三个标签页，使用统一的原生主题样式。
- **LLM Profiles**：保存 LLM provider / profile，把当前 profile 写入 Claude Code settings，并为 OpenAI 代理配置额外请求体字段。
- **OpenAI 兼容代理**：把 Claude Code 的 Anthropic Messages 请求转换到 OpenAI Chat Completions 接口，支持工具调用格式转换、并行工具控制、上下文压缩和历史总结。
- **跨平台**：Windows 提供完整桌面 UI；macOS / Linux 当前运行 headless hook 和 proxy 服务，没有桌面交互 UI。

## 快速开始

开发运行：

```powershell
cargo run --release
```

正常启动会确保 Claude Code hooks 指向当前 claudie 端口；Windows UI 退出时会清理 claudie 管理的 hooks。下面的安装/卸载命令用于手动管理或打包流程。

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

静默模式（抑制安装/卸载时的系统通知弹窗）：

```powershell
cargo run --release -- --install-claude-hooks --quiet
```

## OpenAI 格式 API 代理

claudie 启动时会同时监听本机代理地址：

```text
http://127.0.0.1:17388
```

在 Settings -> LLM Profiles 中新增或编辑一个 profile：

- `Base URL` 填 OpenAI 兼容的 chat completions 地址，例如 `https://example.com/v1/chat/completions`。
- `API key` 填上游 OpenAI 兼容服务的 key。
- `Model` 填该服务支持的模型名。
- `OpenAI body` 可选填额外请求体字段，支持 JSON object 或逐行 `key=value` / `key: value`；例如 `{"reasoning_effort":"xhigh"}` 或 `model_reasoning_effort = "xhigh"`。

点击 `Use` 后，claudie 会把 Claude Code 的 `ANTHROPIC_BASE_URL` 写成上面的本地代理地址；上游请求地址和 key 只保存在 claudie 的 profile 中。Base URL 只要包含 `/chat/completions` 就会自动启用代理；如果填写的是上游根地址，可在 `Extra env` 中加入：

```text
CLAUDIE_API_FORMAT=openai
```

代理当前实现了 Claude Code 常用的 `POST /v1/messages`、`POST /v1/messages/count_tokens` 和 `GET /v1/models`，并支持把工具调用在 Anthropic / OpenAI 格式之间转换。`OpenAI body` 会合并进转发到上游的 chat completions 请求，但不能覆盖 claudie 管理的 `messages` 和 `stream` 字段。

当请求包含工具时，代理默认向上游设置 `parallel_tool_calls=false`，让文件读取、编辑和命令调用按顺序执行，减少多个编辑工具基于过期文件内容并发失败的概率。若确实需要并行工具调用，可在 `OpenAI body` 中显式设置 `{"parallel_tool_calls": true}` 覆盖默认值。

OpenAI 代理默认启用上下文优化。claudie 会在转发请求前压缩超长工具结果和普通文本；当估算输入超过默认阈值时，会保留最近消息并对更早的对话进行分块总结。每个消息块独立生成总结并缓存到 `%USERPROFILE%\.claudie\proxy_cache\` 下的 `summaries/` 和 `chunks/` 目录中。重复请求相同的较早历史时可复用缓存总结，避免再次发起总结请求。缓存只保存总结文本，不保存 API key 或完整原始请求体。

可在 profile 的 `Extra env` 中调整或关闭此行为：

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
```

默认使用本地抽取式总结（`CLAUDIE_PROXY_SUMMARY_MODE=local`），不会为了总结再调用昂贵的上游模型。若希望使用模型生成总结，可设置 `CLAUDIE_PROXY_SUMMARY_MODE=model`；如果模型总结失败，claudie 仍会转发只经过长内容压缩的请求。设置 `CLAUDIE_PROXY_MAX_OUTPUT_TOKENS=0` 可关闭输出 token 上限。

分块总结模式（默认开启）把较早消息按 `CLAUDIE_PROXY_CHUNK_SIZE_MESSAGES` 条一组分块，逐块生成摘要并附加到压缩结果中；设置 `CLAUDIE_PROXY_CHUNK_SUMMARY=0` 可关闭分块、退回单块总括。缓存目录 `proxy_cache/` 下的文件和旧版 `proxy_summaries.json` 均可安全删除，claudie 会在需要时重新生成。

## 项目结构

```text
src/
  main.rs                  CLI 参数、应用启动、hook/profile 初始化和平台入口
  config.rs                端口、尺寸、菜单 ID、颜色和定时常量
  globals.rs               进程级 OnceLock 全局变量
  notifier.rs              平台通知 / 消息框封装
  util.rs                  参数解析、路径、文本截断和 UTF-16 辅助函数
  app/                     AppState、权限请求、选择请求、番茄钟等领域规则
  hooks/                   Claude Code hook server、事件语义、配额提取、settings 合并
  proxy.rs                 本地 Anthropic Messages -> OpenAI Chat Completions 代理
  proxy_optimizer.rs       OpenAI 代理长上下文压缩、分块历史总结和总结缓存
  settings/                用户设置、LLM profiles、Claude env 集成、JSON 存储 helper
  ui/
    gif_animation.rs       GIF 加载、帧延迟读取和 GDI+ 绘制
    theme.rs               Settings 面板和权限/选择弹层共享视觉 token
    window/                主宠物窗口
      mod.rs               窗口生命周期、热键、菜单、点击处理和位置持久化
      render.rs            HUD、宠物、权限弹层和选择卡片绘制
    settings_panel/        原生 Settings 面板
      mod.rs               面板生命周期、tab 切换、保存/刷新逻辑
      controls.rs          Win32 控件创建、文本、字体、消息框 helper
      paint.rs             Settings 面板背景、tab 和字段绘制
```

其它目录：

- `assets/claudie/`：内置宠物 GIF 动画资源。
- `assets/icon.*`、`assets/claudie.manifest`：应用图标和 Windows manifest。
- `packaging/`：Windows/Unix 打包与安装脚本。

## 本地数据

- `%USERPROFILE%\.claudie\settings.json`：宠物资源路径、GIF 目录、动画映射、缩放、睡眠时间、窗口位置和番茄钟设置。
- `%USERPROFILE%\.claudie\llm_profiles.json`：LLM provider/profile 定义，包括 OpenAI 代理额外请求体字段。
- `%USERPROFILE%\.claudie\proxy_summaries.json`：OpenAI 代理旧版（单块）总结缓存。
- `%USERPROFILE%\.claudie\proxy_cache\`：OpenAI 代理缓存目录，包含：
  - `summaries/`：单块总结缓存 JSON 文件。
  - `chunks/`：分块总结缓存 JSON 文件，每块独立缓存。
- `%USERPROFILE%\.claude\settings.json`：Claude Code hook settings 和由 claudie 管理的 LLM env。
- `%USERPROFILE%\.claude\settings.json.claudie.bak`：首次修改 Claude settings 前创建的一次性备份。

## 宠物资源

内置资源位于：

```text
assets/claudie/
  idle.gif
  thinking.gif
  typing.gif
  building.gif
  permission.gif
  happy.gif
  error.gif
  sleeping.gif
  subagent.gif
```

Settings 面板可以调整 GIF 目录和每个 mood 对应的文件名。替换美术资源时保持文件名映射一致即可。

## 维护边界

- `AppState` 是中央可变模型；长期状态和领域规则优先放在 `src/app/`。
- Hook server 保持小而同步；HTTP 解析留在 `src/hooks/server.rs`，Claude event 语义放在 `src/hooks/events.rs`。
- 配额字段兼容逻辑集中在 `src/hooks/quota.rs`。
- 修改 Claude settings 时只合并 claudie 管理的 hook/env 字段，保留用户其它配置。
- 新增 JSON 状态文件时复用 `src/settings/storage.rs`。
- OpenAI 代理上下文优化、长文本压缩、分块历史总结和总结缓存逻辑集中在 `src/proxy_optimizer.rs`。
- UI 线程不要做可能卡顿的网络或文件工作。
- 主窗口新增可视元素优先改 `src/ui/window/render.rs`；新增菜单、热键或鼠标交互优先改 `src/ui/window/mod.rs`。
- Settings 面板和权限/选择弹层的颜色、圆角、字体等共享视觉 token 放在 `src/ui/theme.rs`。
- Settings 面板新增字段时，把窗口消息和保存行为放在 `src/ui/settings_panel/mod.rs`，控件 helper 放在 `controls.rs`，背景/字段装饰放在 `paint.rs`。

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

涉及 UI、hook、权限、settings 或代理行为时，再手动运行：

```powershell
cargo run --release
```

重点检查宠物窗口位置是否会在退出后恢复、右键菜单、Basic/Pomodoro/LLM Profiles tabs、GIF 资源加载、`POST /hook` 状态更新、权限/选择卡片，以及需要时的本地 LLM 代理和 `OpenAI body` 转发。