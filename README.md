# claudie

中文 | [English](README.en.md)

`claudie` 是一个为 Claude Code 设计的轻量桌面宠物。Windows 版本使用 Rust + Win32/GDI+ 原生窗口实现；运行时由桌面 UI、同步 `std::net::TcpListener` hook server，以及本地 Anthropic Messages 兼容代理组成。代理会把 Claude Code 请求转换到 OpenAI Chat Completions 风格的上游服务。

项目刻意避免 Electron、WebView、async runtime 和 Web 框架。宠物资源使用轻量 GIF 目录，每种 mood 映射一个 GIF。

## 致谢

claudie 受 [rullerzhou-afk/clawd-on-desk](https://github.com/rullerzhou-afk/clawd-on-desk) 和 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) 启发。

## 功能

- **Hook 事件驱动**：接收 Claude Code HTTP hooks，并根据事件切换宠物状态。

  | 事件 | 行为 |
  |------|------|
  | `SessionStart` | 回到空闲待命 |
  | `UserPromptSubmit` | 思考中 |
  | `PreToolUse` | 开始执行工具；写入类 -> typing，shell 类 -> building，读取/搜索类 -> search |
  | `PostToolUse` | 工具完成 |
  | `PostToolBatch` | 一批工具完成，并刷新配额快照 |
  | `PostToolUseFailure` / `StopFailure` / `PermissionDenied` | 错误状态 |
  | `PermissionRequest` | 等待用户在宠物 UI 中 Allow / Always / Deny |
  | `SubagentStart` / `TaskCreated` | 子任务进行中 |
  | `SubagentStop` / `TaskCompleted` | 子任务完成 |
  | `PreCompact` / `PostCompact` | 上下文压缩开始 / 完成 |
  | `Notification` / `Elicitation` | 通知提示 |
  | `WorktreeCreate` | 创建 worktree |
  | `Stop` | 任务结束 |
  | `SessionEnd` | 会话结束，清除待处理交互 |

- **权限请求**：通过 `PermissionRequest` hook 接管权限请求，在宠物窗口中显示 Allow / Always / Deny。
- **选择卡片**：支持 `PreToolUse` 中的 `AskUserQuestion` 和 `ExitPlanMode`，显示选项、Submit 和 Cancel。
- **快捷键**：`Ctrl+Shift+Y` 允许权限或提交选择；`Ctrl+Shift+N` 拒绝权限或取消选择。
- **番茄钟**：内置 Pomodoro，支持 Start / Stop / Pause / Resume / Skip，阶段结束时弹出通知。
- **宠物互动**：短按左键播放 `wave` / `stretch` 互动动画，按住移动仍可拖动窗口；专注阶段可使用 `pomodoro` 动画。
- **空闲睡眠**：长时间无活动后进入睡眠，有新活动时唤醒。
- **窗口与资源设置**：支持宠物缩放、窗口位置记忆、GIF 目录和 mood -> GIF 映射。
- **Settings 面板**：Basic、Pomodoro、LLM Profiles、Stats 四个标签页，使用 Slint 原生窗口。
- **LLM Profiles**：保存官方或自定义 LLM profile，可写入 Claude Code settings，并可从右键菜单快速切换。
- **会话小账本**：按天记录 prompts、工具分类、权限/选择、错误、番茄钟完成数和 token 用量；Stats 页展示今日与最近 7 天。
- **OpenAI 兼容代理**：把 Claude Code 的 Anthropic Messages 请求转换到 OpenAI Chat Completions，支持工具调用、流式响应、图片转发、reasoning 输出、并行工具调用、工具历史降级、上下文压缩、历史总结和能力缓存。
- **跨平台**：Windows 提供完整桌面 UI；macOS/Linux 当前只运行 headless hook/proxy 和 CLI hook 管理，没有桌面交互 UI，权限请求会直接拒绝。

## 快速开始

开发运行：

```powershell
cargo run --release
```

正常启动会监听 hook 地址 `http://127.0.0.1:17387/hook` 和本地代理 `http://127.0.0.1:17388`，并确保 Claude Code hooks 指向当前 claudie 端口。Windows UI 退出时会清理 claudie 管理的 hooks。

常用命令：

```powershell
cargo run --release -- --help
cargo run --release -- --port 17387
cargo run --release -- --install-claude-hooks
cargo run --release -- --uninstall-claude-hooks
cargo run --release -- --print-claude-settings
cargo run --release -- --install-claude-hooks --quiet
```

`--install` 和 `--uninstall` 也可作为短别名使用。`--quiet` 会抑制安装/卸载 hook 时的系统通知弹窗。

## OpenAI 兼容 API 代理

claudie 启动时会监听：

```text
http://127.0.0.1:17388
```

在 Settings -> LLM Profiles 中新增或编辑 profile：

- `Base URL` 填上游 OpenAI 兼容地址，例如 `https://example.com/v1/chat/completions` 或 `https://example.com/v1`。
- `API key` 填上游服务 key；如果该字段为空，代理会使用 `Auth token` 作为上游 key。
- `Model` 填上游支持的模型名。
- `OpenAI body` 可选填额外请求体字段，支持 JSON object 或逐行 `key=value` / `key: value`，例如 `{"reasoning_effort":"xhigh"}` 或 `model_reasoning_effort = "xhigh"`。
- `Extra env` 可填每行一个 `KEY=VALUE` 的代理开关或 Claude Code 环境变量。

点击 `Use` 后，如果 profile 是 OpenAI Chat Completions 格式，claudie 会把 Claude Code 的 `ANTHROPIC_BASE_URL` 写成本地代理地址；上游 URL 和 key 只保存在 claudie profile 中。`Base URL` 包含 `/chat/completions` 会自动启用代理；如果填的是上游根地址，可在 `Extra env` 中加入：

```text
CLAUDIE_API_FORMAT=openai
```

代理实现了 `POST /v1/messages`、`POST /v1/messages/count_tokens` 和 `GET /v1/models`。`OpenAI body` 会合并进转发到上游的 chat completions 请求，但不能覆盖 claudie 管理的 `messages` 和 `stream` 字段。

当前代理能力：

- 非流式和流式 OpenAI 响应都会转换回 Anthropic Messages / SSE 事件。
- Anthropic tool use / tool result 会转换为 OpenAI `tools`、`tool_calls` 和 `tool` message。
- 请求包含工具且模型支持工具时，默认发送 `parallel_tool_calls=true`；可用 `{"parallel_tool_calls": false}` 关闭。
- DeepSeek R1、QwQ、GLM-Zero 等 reasoning 流会映射为 Anthropic thinking block。
- OpenAI/Azure/OpenRouter reasoning 模型会自动从 Anthropic `thinking.budget_tokens` 推导 `reasoning_effort`，用户在 `OpenAI body` 中显式设置时优先。
- 支持图片内容转发；默认根据模型名判断 vision 能力，也可用 `CLAUDIE_PROXY_FORWARD_IMAGES=always` 或 `CLAUDIE_PROXY_FORWARD_IMAGES=never` 强制控制。
- 对识别出的 OpenAI/Azure/DeepSeek/Qwen/Kimi/GLM/OpenRouter 上游，兼容提示默认关闭；泛用 OneAPI/NewAPI 类上游默认开启。可用 `CLAUDIE_PROXY_COMPAT_PROMPT=0/1` 控制。
- 若上游拒绝原生工具历史，代理会重试文本 transcript 模式，并把能力探测结果缓存到 `proxy_cache/capabilities/`。

上下文优化默认开启。claudie 会压缩超长工具结果和普通文本；当估算输入超过阈值时，保留最近消息并对较早对话分块总结。缓存只保存摘要文本或能力探测结果，不保存 API key 或完整原始请求体。

可在 profile 的 `Extra env` 中调整：

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
CLAUDIE_PROXY_CAPABILITY_CACHE_TTL_HOURS=720
CLAUDIE_PROXY_CAPABILITY_CACHE_MAX_ENTRIES=200
CLAUDIE_PROXY_COMPAT_PROMPT=0
CLAUDIE_PROXY_FORWARD_IMAGES=auto
```

默认 `CLAUDIE_PROXY_SUMMARY_MODE=local`，使用本地抽取式摘要，不额外调用上游模型。设置 `CLAUDIE_PROXY_SUMMARY_MODE=model` 可改用上游模型摘要；若摘要请求失败，仍会转发经过长内容压缩的请求。设置 `CLAUDIE_PROXY_MAX_OUTPUT_TOKENS=0` 可关闭输出 token 上限。

## 项目结构

```text
src/
  main.rs                  CLI、启动流程、hook/proxy 初始化和平台入口
  config.rs                端口、窗口尺寸、菜单 ID、overlay 几何和常量
  globals.rs               进程级 OnceLock 全局句柄
  notifier.rs              平台通知 / 消息框封装
  util.rs                  参数解析、路径、文本截断和 UTF-16 helper
  app/                     AppState、mood、权限/选择、番茄钟、统计等领域状态
  hooks/                   Claude Code hook server、事件语义、配额提取和 settings 合并
  proxy/                   Anthropic Messages -> OpenAI Chat Completions 代理
  proxy_optimizer/         长上下文压缩、分块摘要和代理缓存
  settings/                用户设置、LLM profiles、Claude env 集成和 JSON 存储
  ui/                      Win32/GDI+ 主窗口、Slint 设置窗/弹窗和渲染逻辑
```

关键文件：

- `src/hooks/events.rs`：hook 事件语义、权限等待、选择响应和 stats 记录。
- `src/hooks/claude_settings.rs`：hook settings 安装、卸载、合并和备份。
- `src/proxy/request_conv.rs` / `response_conv.rs` / `streaming.rs`：请求、响应、流式转换。
- `src/proxy/provider.rs`：provider/model 能力检测、图片转发、reasoning 和 compat prompt 策略。
- `src/proxy/tool_history.rs` / `capability_cache.rs`：工具历史文本降级和能力缓存。
- `src/proxy_optimizer/config.rs` / `compress.rs` / `summary.rs` / `cache.rs`：上下文优化和缓存。
- `src/settings/mod.rs`：profile、OpenAI body、Extra env、Claude settings 写入和路径规范化。
- `src/ui/window/mod.rs`：主窗口生命周期、热键、右键菜单、拖动和 profile 菜单。
- `src/ui/window/render.rs`：HUD、宠物绘制、权限 overlay、选择卡片。
- `src/ui/slint_views.rs` 与 `src/ui/settings_panel/`：Settings / Prompt 窗口声明与控制器。

其它目录：

- `assets/claudie/`：内置 GIF 动画资源。
- `assets/icon.*`、`assets/claudie.manifest`：应用图标和 Windows manifest。
- `packaging/`：Windows/Unix 打包与安装脚本。

## 本地数据

- `%USERPROFILE%\.claudie\settings.json`：资源目录、GIF 映射、缩放、睡眠时间、窗口位置和番茄钟设置。
- `%USERPROFILE%\.claudie\llm_profiles.json`：LLM profiles、active profile、上游 auth、OpenAI body 和 Extra env。
- `%USERPROFILE%\.claudie\daily_stats.json`：每日 prompt、工具分类、权限/选择、错误、focus session 和 token 计数，最多保留 45 天。
- `%USERPROFILE%\.claudie\proxy_summaries.json`：旧版单块摘要缓存。
- `%USERPROFILE%\.claudie\proxy_cache\`：代理缓存目录，包含 `summaries/`、`chunks/` 和 `capabilities/`。
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

Settings 面板可以调整 GIF 目录和每个 mood 对应的文件名。替换资源时保持文件名映射一致即可。

## 打包

Windows 安装包模板位于 `packaging/windows/claudie.iss`：

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
```

输出文件为 `dist\claudie-setup.exe`。Unix 用户级安装脚本在 `packaging/unix/`。

## 验证

提交前至少运行：

```powershell
cargo fmt
cargo check
```

修改 hook settings、配额提取、LLM profile、代理转换/流式、上下文优化、stats、番茄钟或纯领域规则时，也运行：

```powershell
cargo test
```

涉及 UI、hook、权限、settings 或代理行为时，再手动运行：

```powershell
cargo run --release
```

重点检查窗口位置恢复、右键菜单和 LLM Profile 快速切换、Settings 四个 tabs、左键互动与拖动、GIF 加载、`POST /hook` 状态更新、权限/选择卡片、Stats 图表，以及需要时本地代理、流式转换、`OpenAI body` 和上下文优化是否工作。
