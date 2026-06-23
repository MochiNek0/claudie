# claudie

中文 | [English](README.en.md)

> 一只住在 Claude Code 旁边的桌面宠物——它随着 Claude 的工作状态切换动画，在窗口里替你回答权限/选择请求，还能把 Claude Code 接到任意 OpenAI 兼容的模型上。

`claudie` 是一个 **Windows-only** 的轻量桌面宠物，用 **Rust + 原生 Win32/GDI+** 实现，刻意避开 Electron、WebView、async runtime 和 Web 框架——常驻内存小、无浏览器内核。

灵感来自 [rullerzhou-afk/clawd-on-desk](https://github.com/rullerzhou-afk/clawd-on-desk) 和 [farion1231/cc-switch](https://github.com/farion1231/cc-switch)。

## 它由三部分组成

| 组件 | 作用 |
|------|------|
| **桌面宠物 UI** | 根据 Claude Code 活动切换 GIF 动画，并在窗口里直接回答权限/选择请求。 |
| **Hook server** | 同步的 `std::net::TcpListener`，接收 Claude Code 的 HTTP hook 事件（`127.0.0.1:17387/hook`）。 |
| **OpenAI 兼容代理** | 把 Claude Code 的 Anthropic Messages 请求转换为 OpenAI Chat Completions 风格的上游服务（`127.0.0.1:17388`）。 |

## 功能

- **状态随动** — 接收 hook 事件并切换宠物状态：思考、打字、执行命令、搜索、子任务、报错、睡眠……完整映射见下方折叠表。
- **权限请求** — 在宠物窗口显示 Allow / Always / Deny。Deny 写回 `continue=false` + `interrupt=true`，等同终端里回答 “No”：中断本轮，而不是把否决当作可重试的工具反馈喂回模型。终端与宠物两侧任一答复都会同步关闭弹窗。
- **选择卡片** — 支持 `AskUserQuestion`（带「Other…」自由输入）和 `ExitPlanMode`（计划以 Markdown 渲染），可在窗口里选项 + Submit / Cancel。
- **多会话切换** — 跟踪各会话状态，在宠物旁渲染会话切换面板，滚轮切换关注的会话；焦点会话决定宠物 mood 与 HUD。仅单会话时自动隐藏。按 ESC 中断本轮后宠物会回到空闲；已关闭的会话（包括不发 `SessionEnd` 的强制退出）会自动移除。
- **快捷键** — `Ctrl+Shift+Y` 允许 / 提交；`Ctrl+Shift+N` 拒绝 / 取消。
- **国际化** — 内置简体中文与英文双语界面，自动跟随系统语言。`CLAUDIE_LANG=zh` / `CLAUDIE_LANG=en` 可强行指定，所有 UI 文本统一切换。
- **番茄钟** — 内置 Pomodoro，支持 Start / Stop / Pause / Resume / Skip，阶段结束弹通知。
- **钓鱼小游戏** — 点击宠物开始「等待 → 提竿 → 收线 → 上钩/脱钩」，收线阶段需在移动目标区维持张力。
- **官方用量监控** — 右键菜单与设置面板实时显示官方 5 小时 / 7 天用量（OAuth 轮询），识别 Max/Pro/Team 订阅与重置倒计时。
- **LLM Profiles** — 保存官方或自定义 profile，写入 Claude Code settings，右键菜单一键切换。
- **OpenAI 兼容代理** — 工具调用、流式响应、图片转发、reasoning 输出、并行工具调用、工具历史降级、上下文压缩与历史总结。详见[下文](#openai-兼容代理)。
- **本地小账本** — 按天记录 prompts、工具分类、权限/选择、错误、番茄钟完成数和 token 用量，Stats 页展示今日与近 7 天，数据不出本机。
- **其它** — 系统托盘图标、宠物缩放与窗口位置记忆、短按互动动画、空闲自动睡眠、Secrets（API key / OAuth token）DPAPI 加密存储。

<details>
<summary>Hook 事件 → 宠物状态对照表</summary>

| 事件 | 行为 |
|------|------|
| `SessionStart` | 回到空闲待命 |
| `UserPromptSubmit` | 思考中 |
| `PreToolUse` | 开始执行工具；写入类 → typing，shell 类 → building，读取/搜索类 → search |
| `PostToolUse` | 工具完成 |
| `PostToolBatch` | 一批工具完成，并刷新配额快照 |
| `PostToolUseFailure` | 耸肩 shrug（可恢复的小失败，独立 mood，低优先级，很快被下个工具覆盖） |
| `StopFailure` | 错误状态 |
| `PermissionDenied` | 拒绝状态（独立 mood） |
| `PermissionRequest` | 等待用户在宠物 UI 中 Allow / Always / Deny |
| `SubagentStart` / `TaskCreated` | 子任务进行中 |
| `SubagentStop` / `TaskCompleted` | 子任务完成 |
| `PreCompact` / `PostCompact` | 上下文压缩开始 / 完成 |
| `Notification` / `Elicitation` | 通知提示 |
| `WorktreeCreate` | 创建 worktree |
| `Stop` | 任务结束 |
| `SessionEnd` | 会话结束，清除待处理交互 |

</details>

## 快速开始

环境要求：Windows 10/11 + [Rust 工具链](https://rustup.rs/)（仅支持 Windows，非 Windows 不编译）。

```powershell
cargo run --release
```

正常启动会监听 hook (`:17387`) 和代理 (`:17388`)，并自动把 Claude Code hooks 指向当前端口；UI 退出时清理 claudie 管理的 hooks。无需手动安装 hook 即可使用。

常用命令（`--install` / `--uninstall` 为短别名，`--quiet` 抑制安装通知弹窗）：

```powershell
cargo run --release -- --help
cargo run --release -- --port 17387
cargo run --release -- --install-claude-hooks      # 别名：--install
cargo run --release -- --uninstall-claude-hooks    # 别名：--uninstall
cargo run --release -- --print-claude-settings
```

打成安装包给非开发者使用见 [打包](#打包)。

## OpenAI 兼容代理

代理监听 `http://127.0.0.1:17388`，实现 `POST /v1/messages`、`POST /v1/messages/count_tokens` 和 `GET /v1/models`，让 Claude Code 跑在任意 OpenAI 兼容上游（DeepSeek、Qwen、Kimi、GLM、OpenRouter、OneAPI/NewAPI 等）上。

在 **Settings → LLM Profiles** 中配置 profile：

| 字段 | 说明 |
|------|------|
| `Base URL` | 上游 OpenAI 兼容地址，如 `https://example.com/v1/chat/completions` 或 `https://example.com/v1`。 |
| `API key` | 上游服务 key；留空则用 `Auth token` 作为上游 key。 |
| `Model` | 上游支持的模型名。每个模型行带一个「1M」开关：勾选后写入 Claude Code 的模型 id 会附加 `[1m]` 后缀，代理转发前自动剥除。Models 区顶部的 `Fetch models` 按钮会按 OpenAI `/v1/models` 抓取上游模型列表（兼容 `/anthropic` 等子路径与 `/v{N}` 版本段），抓取后的下拉框可直接选模型。 |
| `OpenAI body` | 可选额外请求体，JSON object 或逐行 `key=value` / `key: value`，如 `{"reasoning_effort":"xhigh"}`。会合并进上游请求，但不能覆盖 claudie 管理的 `messages` / `stream`。 |
| `Extra env` | 每行一个 `KEY=VALUE` 的代理开关或 Claude Code 环境变量。 |

点击 `Use` 后，若 profile 是 OpenAI 格式，claudie 会把 Claude Code 的 `ANTHROPIC_BASE_URL` 指向本地代理（上游 URL/key 只留在 claudie profile 中）。`Base URL` 含 `/chat/completions` 会自动启用代理；若填的是上游根地址，在 `Extra env` 加 `CLAUDIE_API_FORMAT=openai`。

**代理能力**：流式与非流式 OpenAI ↔ Anthropic Messages / SSE 互转；工具调用映射；默认支持 `parallel_tool_calls`；DeepSeek/QwQ/GLM-Zero 等 reasoning 流映射为 Anthropic thinking 块，OpenAI/Azure/OpenRouter 自动推导 `reasoning_effort`；图片转发自动识别；主流上游默认关闭兼容提示；上游拒绝工具历史时自动降级为文本 transcript；429/529 的 `Retry-After` 透传，临时错误返回 HTTP 529。

**上下文优化**（默认开启）：压缩超长工具结果和文本；输入超阈值时保留最近消息、对较早对话做本地抽取式分块总结（不调用上游）。缓存只保存分块摘要和能力探测结果，不保存 API key 或原始请求体。

<details>
<summary><code>Extra env</code> 可调参数（默认值）</summary>CLAUDIE_PROXY_OPTIMIZE=1 / SUMMARY_THRESHOLD=24000 / KEEP_RECENT_MESSAGES=12 / KEEP_RECENT_TOKENS=10000 / TOOL_RESULT_LIMIT=3000 / TEXT_LIMIT=6000 / LOCAL_SUMMARY_TOKENS=2000 / CACHE_MAX_MB=10 / CHUNK_SIZE_MESSAGES=8 / COMPAT_PROMPT=0 / FORWARD_IMAGES=auto
</details>

## Stats 面板

Settings → Stats 基于本地 `daily_stats.json`（最多 45 天）展示使用情况，数据不留出本机。顶部 KPI 大字为今日值，下方 `7d · N` 为近 7 天合计，涵盖 Prompts、Tokens（输入+输出+缓存读写）、Cache hit、Tool calls。Activity 为 14 天柱状图；Productivity highlights、Tool mix 与 Token 分布见 Detail 区域。

## 本地数据

均位于 `%USERPROFILE%\.claudie\`（除最后两项在 `.claude\`）：

| 文件 | 内容 |
|------|------|
| `settings.json` | 资源目录、GIF 映射、缩放、睡眠时间、窗口位置、番茄钟设置 |
| `llm_profiles.json` | LLM profiles、active profile、上游 auth、OpenAI body、Extra env |
| `secrets.json` | DPAPI 加密的敏感凭据（API key、OAuth token），仅当前 Windows 用户可解密 |
| `daily_stats.json` | 每日计数（prompt、工具、权限/选择、错误、focus、token），保留 45 天 |
| `proxy_cache/` | 代理缓存：`chunks/`、`capabilities/` |
| `.claude\settings.json` | Claude Code hook settings 和 claudie 管理的 LLM env |
| `.claude\settings.json.claudie.bak` | 首次修改前的一次性备份 |

## 宠物资源

内置 GIF 位于 `assets/claudie/`，每种 mood 一个文件：`idle` `thinking` `typing` `building` `search` `happy` `error` `deny` `sleeping` `subagent` `pomodoro` `wave` `stretch` `fishing` `reel` `caught` `missed`。在 Settings 面板可调整 GIF 目录与各 mood 的文件名，替换素材时保持映射一致即可。

## 打包

Windows 安装包模板位于 `packaging/windows/claudie.iss`，输出 `dist\claudie-setup.exe`：

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
```

## 发布

推送 `v*` 标签即由 GitHub Actions（`.github/workflows/release.yml`）自动编译、打包安装器并创建 GitHub Release，附带 `claudie-setup.exe`。发布步骤：

1. 修改 `Cargo.toml` 的 `version`（应用通过 `CARGO_PKG_VERSION` 上报版本，需与标签一致，否则 CI 失败）。
2. 提交并推送。
3. `git tag vX.Y.Z && git push origin vX.Y.Z`。

应用会每 24 小时静默检查 `MochiNek0/claudie` 的最新 Release，发现新版本时右键菜单出现「发现新版本 vX.Y.Z」项，点击用浏览器打开下载页。

## 开发

提交前至少运行 `cargo fmt` 和 `cargo check`；触及 hook、配额、profile、代理转换/流式、优化器、stats、番茄钟等纯领域逻辑时运行 `cargo test`；UI/hook/权限/代理行为改动还需 `cargo run --release` 手动验证。

代码结构、关键文件清单和完整验证清单见 [AGENTS.md](AGENTS.md)。
